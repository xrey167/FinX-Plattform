//! The mutable `WorkspaceState` dashboard/layout document the control plane edits.
//!
//! This is the document model behind every `tdw.workspace.*` tool: a set of named
//! dashboards, each a tab carrying a grid `layout` of placed widgets, plus an
//! "active" selection (the navigated dashboard). It mirrors the `apps.json` shape
//! projected by [`tdw_widgets`] — a [`WorkspaceState`] serializes losslessly to a
//! [`tdw_widgets::AppConfig`] (one tab per dashboard) so the produced layout is a
//! valid Workspace app.
//!
//! # Invariants enforced here (not by the Workspace frontend)
//!
//! - **Catalog-validated widgets.** A widget id placed on the grid must exist in
//!   the [`tdw_widgets::catalog_widgets`] catalog. An unknown id is rejected.
//! - **Bounded, non-overlapping grid.** The grid is [`GRID_COLUMNS`] columns wide
//!   (the `apps.json` two-pane convention is two 20-wide widgets). A placement
//!   must start in bounds, fit within the column count, have positive size, and
//!   not overlap any other widget on the same dashboard.
//! - **Stable instance ids.** Each placed widget gets a unique instance id so the
//!   same catalog widget can appear twice on a dashboard and still be addressed
//!   for move/resize/remove.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_widgets::{AppConfig, AppTab, LayoutItem, McpServerRef};

/// Width of the Workspace grid in columns.
///
/// The curated `apps.json` default app composes two 20-column widgets per row, so
/// a full row is 40 columns. A placement's `x + w` must not exceed this.
pub const GRID_COLUMNS: u32 = 40;

/// Error returned by a control-plane mutation that violates a document invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    /// A dashboard with the given id already exists.
    DashboardExists(String),
    /// No dashboard with the given id exists.
    DashboardNotFound(String),
    /// The widget id is not present in the `tdw-widgets` catalog.
    UnknownWidget(String),
    /// No placed widget instance with the given instance id exists on the dashboard.
    WidgetInstanceNotFound(String),
    /// The placement starts or extends outside the bounded grid.
    OutOfBounds {
        /// Human-readable detail (which edge was exceeded).
        detail: String,
    },
    /// A zero-width or zero-height placement (a widget must occupy at least 1x1).
    ZeroSize,
    /// The placement overlaps an already-placed widget instance.
    Overlap {
        /// Instance id of the widget already occupying the contested cells.
        with: String,
    },
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DashboardExists(id) => write!(f, "dashboard already exists: {id}"),
            Self::DashboardNotFound(id) => write!(f, "dashboard not found: {id}"),
            Self::UnknownWidget(id) => write!(f, "unknown widget id (not in catalog): {id}"),
            Self::WidgetInstanceNotFound(id) => write!(f, "widget instance not found: {id}"),
            Self::OutOfBounds { detail } => write!(f, "placement out of bounds: {detail}"),
            Self::ZeroSize => write!(f, "placement must be at least 1x1"),
            Self::Overlap { with } => write!(f, "placement overlaps widget instance: {with}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

/// A rectangular grid placement: origin `(x, y)` and size `(w, h)` in grid cells.
///
/// Coordinates are 0-based from the top-left, matching the `apps.json`
/// [`tdw_widgets::LayoutItem`] model (`x`/`y` origin, `w`/`h` span).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// Column origin (0-based).
    pub x: u32,
    /// Row origin (0-based).
    pub y: u32,
    /// Width in columns (must be >= 1).
    pub w: u32,
    /// Height in rows (must be >= 1).
    pub h: u32,
}

impl Placement {
    /// Whether this placement shares any grid cell with `other`.
    #[must_use]
    const fn intersects(self, other: Self) -> bool {
        let x_overlap = self.x < other.x + other.w && other.x < self.x + self.w;
        let y_overlap = self.y < other.y + other.h && other.y < self.y + self.h;
        x_overlap && y_overlap
    }

    /// Validate the placement against the bounded grid: positive size and within
    /// the [`GRID_COLUMNS`] column span (height is unbounded — rows stack).
    fn validate_bounds(self) -> Result<(), WorkspaceError> {
        if self.w == 0 || self.h == 0 {
            return Err(WorkspaceError::ZeroSize);
        }
        // `x + w` can overflow only for absurd inputs; saturating keeps the check
        // honest (a saturated value still exceeds GRID_COLUMNS and is rejected).
        if self.x.saturating_add(self.w) > GRID_COLUMNS {
            return Err(WorkspaceError::OutOfBounds {
                detail: format!(
                    "x={} + w={} exceeds grid width {GRID_COLUMNS}",
                    self.x, self.w
                ),
            });
        }
        if self.x >= GRID_COLUMNS {
            return Err(WorkspaceError::OutOfBounds {
                detail: format!("x={} is past the last column {}", self.x, GRID_COLUMNS - 1),
            });
        }
        Ok(())
    }
}

/// One placed widget instance on a dashboard: a catalog widget id at a grid spot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedWidget {
    /// Unique-per-dashboard instance id (so one widget id can appear twice).
    pub instance_id: String,
    /// Catalog widget id, e.g. `equity_price_historical`. Validated on insertion.
    pub widget_id: String,
    /// Grid placement.
    pub placement: Placement,
}

/// A single dashboard: a named tab carrying a grid layout of placed widgets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dashboard {
    /// Stable dashboard id (also the `apps.json` tab id).
    pub id: String,
    /// Human-facing name.
    pub name: String,
    /// Placed widgets, in insertion order.
    pub widgets: Vec<PlacedWidget>,
    /// Monotonic counter feeding unique instance ids (never decremented).
    next_instance: u64,
}

impl Dashboard {
    /// A fresh, empty dashboard.
    fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            widgets: Vec::new(),
            next_instance: 0,
        }
    }

    /// Locate a placed widget instance by its instance id.
    fn find(&self, instance_id: &str) -> Option<&PlacedWidget> {
        self.widgets.iter().find(|w| w.instance_id == instance_id)
    }

    /// Whether `placement` overlaps any placed widget other than `skip` (the
    /// instance being moved/resized, which must not collide with itself).
    fn overlap(&self, placement: Placement, skip: Option<&str>) -> Option<String> {
        self.widgets
            .iter()
            .filter(|w| Some(w.instance_id.as_str()) != skip)
            .find(|w| w.placement.intersects(placement))
            .map(|w| w.instance_id.clone())
    }
}

/// The full control-plane document: dashboards plus the active selection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// Workspace app name (the `apps.json` top-level key / `name`).
    pub name: String,
    /// Workspace app description.
    pub description: String,
    /// Dashboards keyed by id (deterministic order for stable serialization).
    dashboards: BTreeMap<String, Dashboard>,
    /// Id of the currently-navigated (active) dashboard, if any.
    active: Option<String>,
}

impl WorkspaceState {
    /// A new, empty workspace document with the given app name/description.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            dashboards: BTreeMap::new(),
            active: None,
        }
    }

    /// Whether `widget_id` exists in the `tdw-widgets` catalog.
    #[must_use]
    pub fn is_known_widget(widget_id: &str) -> bool {
        tdw_widgets::catalog_widgets()
            .iter()
            .any(|w| w.id == widget_id)
    }

    /// The id of the active (navigated) dashboard, if one is selected.
    #[must_use]
    pub fn active_dashboard(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Dashboard ids in deterministic (sorted) order.
    #[must_use]
    pub fn dashboard_ids(&self) -> Vec<String> {
        self.dashboards.keys().cloned().collect()
    }

    /// Borrow a dashboard by id.
    #[must_use]
    pub fn dashboard(&self, id: &str) -> Option<&Dashboard> {
        self.dashboards.get(id)
    }

    /// Number of dashboards.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dashboards.len()
    }

    /// Whether the workspace has no dashboards.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dashboards.is_empty()
    }

    /// Create a new, empty dashboard. The first dashboard created becomes active.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::DashboardExists`] if a dashboard with `id` already exists.
    pub fn create_dashboard(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<(), WorkspaceError> {
        let id = id.into();
        if self.dashboards.contains_key(&id) {
            return Err(WorkspaceError::DashboardExists(id));
        }
        let dashboard = Dashboard::new(id.clone(), name);
        self.dashboards.insert(id.clone(), dashboard);
        if self.active.is_none() {
            self.active = Some(id);
        }
        Ok(())
    }

    /// Delete a dashboard. If it was active, the active selection moves to the
    /// first remaining dashboard (or `None` when none remain).
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::DashboardNotFound`] if no dashboard with `id` exists.
    pub fn delete_dashboard(&mut self, id: &str) -> Result<(), WorkspaceError> {
        if self.dashboards.remove(id).is_none() {
            return Err(WorkspaceError::DashboardNotFound(id.to_string()));
        }
        if self.active.as_deref() == Some(id) {
            self.active = self.dashboards.keys().next().cloned();
        }
        Ok(())
    }

    /// Rename a dashboard (its display name; the id is stable).
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::DashboardNotFound`] if no dashboard with `id` exists.
    pub fn rename_dashboard(
        &mut self,
        id: &str,
        name: impl Into<String>,
    ) -> Result<(), WorkspaceError> {
        let dashboard = self
            .dashboards
            .get_mut(id)
            .ok_or_else(|| WorkspaceError::DashboardNotFound(id.to_string()))?;
        dashboard.name = name.into();
        Ok(())
    }

    /// Select the active dashboard.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::DashboardNotFound`] if `id` names no dashboard.
    pub fn navigate(&mut self, id: &str) -> Result<(), WorkspaceError> {
        if !self.dashboards.contains_key(id) {
            return Err(WorkspaceError::DashboardNotFound(id.to_string()));
        }
        self.active = Some(id.to_string());
        Ok(())
    }

    /// Add a catalog widget to a dashboard at `placement`, returning the new
    /// instance id.
    ///
    /// # Errors
    ///
    /// - [`WorkspaceError::DashboardNotFound`] if the dashboard is unknown.
    /// - [`WorkspaceError::UnknownWidget`] if `widget_id` is not in the catalog.
    /// - [`WorkspaceError::ZeroSize`] / [`WorkspaceError::OutOfBounds`] for an
    ///   invalid placement, or [`WorkspaceError::Overlap`] when it collides with a
    ///   placed widget.
    pub fn add_widget(
        &mut self,
        dashboard_id: &str,
        widget_id: &str,
        placement: Placement,
    ) -> Result<String, WorkspaceError> {
        if !Self::is_known_widget(widget_id) {
            return Err(WorkspaceError::UnknownWidget(widget_id.to_string()));
        }
        placement.validate_bounds()?;
        let dashboard = self
            .dashboards
            .get_mut(dashboard_id)
            .ok_or_else(|| WorkspaceError::DashboardNotFound(dashboard_id.to_string()))?;
        if let Some(with) = dashboard.overlap(placement, None) {
            return Err(WorkspaceError::Overlap { with });
        }
        let instance_id = format!("{widget_id}-{}", dashboard.next_instance);
        dashboard.next_instance += 1;
        dashboard.widgets.push(PlacedWidget {
            instance_id: instance_id.clone(),
            widget_id: widget_id.to_string(),
            placement,
        });
        Ok(instance_id)
    }

    /// Move a placed widget instance to a new origin, keeping its size.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::DashboardNotFound`] / [`WorkspaceError::WidgetInstanceNotFound`]
    /// for unknown targets, or the placement errors when the destination is out of
    /// bounds or overlaps another widget.
    pub fn move_widget(
        &mut self,
        dashboard_id: &str,
        instance_id: &str,
        x: u32,
        y: u32,
    ) -> Result<(), WorkspaceError> {
        let current = self.placement_of(dashboard_id, instance_id)?;
        let moved = Placement {
            x,
            y,
            w: current.w,
            h: current.h,
        };
        self.replace_placement(dashboard_id, instance_id, moved)
    }

    /// Resize a placed widget instance, keeping its origin.
    ///
    /// # Errors
    ///
    /// As [`move_widget`](Self::move_widget): unknown target, out-of-bounds, or
    /// overlap.
    pub fn resize_widget(
        &mut self,
        dashboard_id: &str,
        instance_id: &str,
        w: u32,
        h: u32,
    ) -> Result<(), WorkspaceError> {
        let current = self.placement_of(dashboard_id, instance_id)?;
        let resized = Placement {
            x: current.x,
            y: current.y,
            w,
            h,
        };
        self.replace_placement(dashboard_id, instance_id, resized)
    }

    /// Remove a placed widget instance from a dashboard.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::DashboardNotFound`] / [`WorkspaceError::WidgetInstanceNotFound`].
    pub fn remove_widget(
        &mut self,
        dashboard_id: &str,
        instance_id: &str,
    ) -> Result<(), WorkspaceError> {
        let dashboard = self
            .dashboards
            .get_mut(dashboard_id)
            .ok_or_else(|| WorkspaceError::DashboardNotFound(dashboard_id.to_string()))?;
        let before = dashboard.widgets.len();
        dashboard.widgets.retain(|w| w.instance_id != instance_id);
        if dashboard.widgets.len() == before {
            return Err(WorkspaceError::WidgetInstanceNotFound(
                instance_id.to_string(),
            ));
        }
        Ok(())
    }

    /// Current placement of an instance (helper for move/resize).
    fn placement_of(
        &self,
        dashboard_id: &str,
        instance_id: &str,
    ) -> Result<Placement, WorkspaceError> {
        self.dashboards
            .get(dashboard_id)
            .ok_or_else(|| WorkspaceError::DashboardNotFound(dashboard_id.to_string()))?
            .find(instance_id)
            .map(|w| w.placement)
            .ok_or_else(|| WorkspaceError::WidgetInstanceNotFound(instance_id.to_string()))
    }

    /// Validate and commit a new placement for an existing instance.
    fn replace_placement(
        &mut self,
        dashboard_id: &str,
        instance_id: &str,
        placement: Placement,
    ) -> Result<(), WorkspaceError> {
        placement.validate_bounds()?;
        let dashboard = self
            .dashboards
            .get_mut(dashboard_id)
            .ok_or_else(|| WorkspaceError::DashboardNotFound(dashboard_id.to_string()))?;
        if dashboard.find(instance_id).is_none() {
            return Err(WorkspaceError::WidgetInstanceNotFound(
                instance_id.to_string(),
            ));
        }
        if let Some(with) = dashboard.overlap(placement, Some(instance_id)) {
            return Err(WorkspaceError::Overlap { with });
        }
        if let Some(widget) = dashboard
            .widgets
            .iter_mut()
            .find(|w| w.instance_id == instance_id)
        {
            widget.placement = placement;
        }
        Ok(())
    }

    /// Project this document to the `apps.json`-shaped [`AppConfig`].
    ///
    /// Each dashboard becomes one [`AppTab`] whose `layout` is the placed widgets
    /// as [`LayoutItem`]s (`i` = catalog widget id, `x`/`y`/`w`/`h` = placement).
    /// The result serializes (via [`tdw_widgets`]) to the exact `apps.json` shape
    /// a Workspace app loads.
    #[must_use]
    pub fn to_app_config(&self) -> AppConfig {
        let mut tabs = BTreeMap::new();
        for (id, dashboard) in &self.dashboards {
            let layout = dashboard
                .widgets
                .iter()
                .map(|w| LayoutItem {
                    i: w.widget_id.clone(),
                    x: w.placement.x,
                    y: w.placement.y,
                    w: w.placement.w,
                    h: w.placement.h,
                })
                .collect();
            tabs.insert(
                id.clone(),
                AppTab {
                    id: id.clone(),
                    name: dashboard.name.clone(),
                    layout,
                },
            );
        }
        AppConfig {
            name: self.name.clone(),
            description: self.description.clone(),
            img: None,
            tabs,
            prompts: Vec::new(),
            mcp_servers: vec![McpServerRef {
                name: "tdw-workspace-mcp".to_string(),
                url: "stdio".to_string(),
            }],
        }
    }

    /// Serialize the current layout to the `apps.json` shape (one app keyed by
    /// its name), matching [`tdw_widgets::apps_json`]'s envelope.
    #[must_use]
    pub fn to_apps_json(&self) -> Value {
        let app = self.to_app_config();
        let mut map = serde_json::Map::new();
        if let Ok(value) = serde_json::to_value(&app) {
            map.insert(app.name, value);
        }
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real catalog widget id, taken from the `tdw-widgets` catalog so the test
    /// stays in lockstep with the projected catalog.
    fn a_real_widget_id() -> String {
        tdw_widgets::catalog_widgets()
            .first()
            .expect("catalog is non-empty")
            .id
            .clone()
    }

    fn place(x: u32, y: u32, w: u32, h: u32) -> Placement {
        Placement { x, y, w, h }
    }

    #[test]
    fn create_first_dashboard_becomes_active() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        assert_eq!(ws.active_dashboard(), Some("main"));
        assert_eq!(ws.len(), 1);
    }

    #[test]
    fn duplicate_dashboard_id_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        assert_eq!(
            ws.create_dashboard("main", "Again"),
            Err(WorkspaceError::DashboardExists("main".to_string()))
        );
    }

    #[test]
    fn add_unknown_widget_id_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let err = ws
            .add_widget("main", "definitely_not_a_real_widget", place(0, 0, 10, 10))
            .expect_err("unknown widget must be rejected");
        assert_eq!(
            err,
            WorkspaceError::UnknownWidget("definitely_not_a_real_widget".to_string())
        );
    }

    #[test]
    fn add_catalog_validated_widget_succeeds() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        let instance = ws
            .add_widget("main", &id, place(0, 0, 20, 12))
            .expect("known widget places");
        assert!(instance.starts_with(&id));
        assert_eq!(ws.dashboard("main").expect("dash").widgets.len(), 1);
    }

    #[test]
    fn add_out_of_bounds_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        // x=30 + w=20 = 50 > GRID_COLUMNS (40).
        let err = ws
            .add_widget("main", &id, place(30, 0, 20, 12))
            .expect_err("out of bounds must be rejected");
        assert!(matches!(err, WorkspaceError::OutOfBounds { .. }));
    }

    #[test]
    fn add_zero_size_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        assert_eq!(
            ws.add_widget("main", &id, place(0, 0, 0, 12)),
            Err(WorkspaceError::ZeroSize)
        );
    }

    #[test]
    fn overlapping_add_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        let first = ws
            .add_widget("main", &id, place(0, 0, 20, 12))
            .expect("first places");
        let err = ws
            .add_widget("main", &id, place(10, 6, 20, 12))
            .expect_err("overlapping placement must be rejected");
        assert_eq!(err, WorkspaceError::Overlap { with: first });
    }

    #[test]
    fn adjacent_non_overlapping_add_succeeds() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        ws.add_widget("main", &id, place(0, 0, 20, 12))
            .expect("left places");
        // Starts exactly where the first ends (x=20): edge-adjacent, no overlap.
        ws.add_widget("main", &id, place(20, 0, 20, 12))
            .expect("right places adjacent");
        assert_eq!(ws.dashboard("main").expect("dash").widgets.len(), 2);
    }

    #[test]
    fn move_into_overlap_is_rejected_but_self_move_ok() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        let left = ws
            .add_widget("main", &id, place(0, 0, 20, 12))
            .expect("left");
        let right = ws
            .add_widget("main", &id, place(20, 0, 20, 12))
            .expect("right");
        // Moving `right` onto `left` collides.
        assert_eq!(
            ws.move_widget("main", &right, 0, 0),
            Err(WorkspaceError::Overlap { with: left })
        );
        // Moving `right` within bounds to a free spot (and onto its own old cells)
        // is fine — it must not collide with itself.
        ws.move_widget("main", &right, 20, 0).expect("self move ok");
    }

    #[test]
    fn resize_into_overlap_and_out_of_bounds_are_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        let left = ws.add_widget("main", &id, place(0, 0, 20, 12)).expect("l");
        let right = ws.add_widget("main", &id, place(20, 0, 20, 12)).expect("r");
        // Growing `left` to width 30 would reach into `right`.
        assert_eq!(
            ws.resize_widget("main", &left, 30, 12),
            Err(WorkspaceError::Overlap {
                with: right.clone()
            })
        );
        // Growing `right` past the grid edge is out of bounds.
        assert!(matches!(
            ws.resize_widget("main", &right, 30, 12),
            Err(WorkspaceError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn move_resize_remove_unknown_instance_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        assert_eq!(
            ws.move_widget("main", "ghost", 0, 0),
            Err(WorkspaceError::WidgetInstanceNotFound("ghost".to_string()))
        );
        assert_eq!(
            ws.resize_widget("main", "ghost", 1, 1),
            Err(WorkspaceError::WidgetInstanceNotFound("ghost".to_string()))
        );
        assert_eq!(
            ws.remove_widget("main", "ghost"),
            Err(WorkspaceError::WidgetInstanceNotFound("ghost".to_string()))
        );
    }

    #[test]
    fn navigate_unknown_dashboard_is_rejected() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("main", "Main").expect("create");
        assert_eq!(
            ws.navigate("nope"),
            Err(WorkspaceError::DashboardNotFound("nope".to_string()))
        );
        assert_eq!(ws.active_dashboard(), Some("main"));
    }

    #[test]
    fn delete_active_dashboard_reselects() {
        let mut ws = WorkspaceState::new("App", "desc");
        ws.create_dashboard("a", "A").expect("a");
        ws.create_dashboard("b", "B").expect("b");
        assert_eq!(ws.active_dashboard(), Some("a"));
        ws.delete_dashboard("a").expect("delete");
        assert_eq!(ws.active_dashboard(), Some("b"));
        ws.delete_dashboard("b").expect("delete");
        assert_eq!(ws.active_dashboard(), None);
        assert!(ws.is_empty());
    }

    #[test]
    fn full_crud_lifecycle() {
        let mut ws = WorkspaceState::new("Lifecycle", "round-trip");
        ws.create_dashboard("main", "Main").expect("create");
        let id = a_real_widget_id();
        let instance = ws
            .add_widget("main", &id, place(0, 0, 20, 12))
            .expect("add");
        ws.move_widget("main", &instance, 20, 0).expect("move");
        ws.resize_widget("main", &instance, 20, 24).expect("resize");
        ws.rename_dashboard("main", "Renamed").expect("rename");
        assert_eq!(ws.dashboard("main").expect("dash").name, "Renamed");
        ws.remove_widget("main", &instance).expect("remove");
        assert!(ws.dashboard("main").expect("dash").widgets.is_empty());
        ws.delete_dashboard("main").expect("delete");
        assert!(ws.is_empty());
    }

    #[test]
    fn layout_round_trips_to_apps_json_shape() {
        let mut ws = WorkspaceState::new("Round Trip", "apps.json shape");
        ws.create_dashboard("overview", "Overview").expect("create");
        let id = a_real_widget_id();
        ws.add_widget("overview", &id, place(0, 0, 20, 12))
            .expect("add");

        let apps = ws.to_apps_json();
        let obj = apps
            .as_object()
            .expect("apps.json is an object keyed by name");
        let app = obj.get("Round Trip").expect("app keyed by its name");
        assert_eq!(app.get("name").and_then(Value::as_str), Some("Round Trip"));

        // The tab carries the placed widget as an apps.json LayoutItem.
        let tab = app
            .get("tabs")
            .and_then(|t| t.get("overview"))
            .expect("overview tab present");
        let layout = tab
            .get("layout")
            .and_then(Value::as_array)
            .expect("layout array");
        assert_eq!(layout.len(), 1);
        assert_eq!(
            layout[0].get("i").and_then(Value::as_str),
            Some(id.as_str())
        );
        assert_eq!(layout[0].get("x").and_then(Value::as_u64), Some(0));
        assert_eq!(layout[0].get("w").and_then(Value::as_u64), Some(20));

        // And the whole document deserializes back into the tdw-widgets AppConfig
        // (i.e. it really is the apps.json shape, not a look-alike).
        let app_config: AppConfig =
            serde_json::from_value(app.clone()).expect("apps.json deserializes to AppConfig");
        assert_eq!(app_config.tabs.len(), 1);
        assert_eq!(app_config.tabs["overview"].layout.len(), 1);
    }
}
