//! Shared bounded-turn primitives.
//!
//! A long-running agent or fetch loop needs a small, dependency-free vocabulary
//! for *why it stopped* and *how much budget it has left*. This module supplies
//! exactly that:
//!
//! - [`TerminalReason`] — the discrete reasons a bounded turn can end.
//! - [`TurnOutcome`] — a terminal reason paired with the iteration count.
//! - [`IterationBudget`] — a simple "at most N iterations" counter whose
//!   [`tick`](IterationBudget::tick) refuses to overrun.
//! - [`CostBudget`] — an independent token-spend cap (a second, orthogonal stop
//!   condition) whose [`add`](CostBudget::add) refuses to overrun.
//!
//! Everything here is pure: no async, no I/O, and no dependencies beyond the
//! standard library. `tdw-core` is the foundational base crate that the entire
//! provider tree depends on, so this module deliberately stays dependency-free.
//! In particular [`CostBudget::add`] takes raw `u32` token counts rather than a
//! borrowed usage type from any LLM crate — callers that hold such a type simply
//! pass its `input_tokens` / `output_tokens` fields.

/// Why a bounded turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerminalReason {
    /// The work finished on its own before hitting any limit.
    NaturalStop,
    /// The iteration budget was exhausted (see [`IterationBudget`]).
    IterationLimitReached,
    /// The turn was cancelled by an external signal.
    Cancelled,
    /// A spend budget was exhausted (see [`CostBudget`]).
    BudgetExhausted,
    /// The turn ended because of an unrecoverable error.
    FatalError,
}

impl TerminalReason {
    /// A stable, lowercase-ish identifier suitable for logs and metrics labels.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NaturalStop => "natural_stop",
            Self::IterationLimitReached => "iteration_limit_reached",
            Self::Cancelled => "cancelled",
            Self::BudgetExhausted => "budget_exhausted",
            Self::FatalError => "fatal_error",
        }
    }
}

/// The result of running a bounded turn: why it stopped and how many iterations
/// it took to get there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TurnOutcome {
    /// The reason the turn ended.
    pub reason: TerminalReason,
    /// The number of iterations that were consumed.
    pub iterations: u32,
}

impl TurnOutcome {
    /// Construct a [`TurnOutcome`] from its parts.
    #[must_use]
    pub fn new(reason: TerminalReason, iterations: u32) -> Self {
        Self { reason, iterations }
    }
}

/// An "at most `max` iterations" counter.
///
/// Call [`tick`](Self::tick) once at the top of each iteration. It returns
/// `Ok(())` while budget remains and `Err(TerminalReason::IterationLimitReached)`
/// once the cap is hit, without ever incrementing past `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IterationBudget {
    max: u32,
    used: u32,
}

impl IterationBudget {
    /// Create a budget that permits at most `max` iterations.
    #[must_use]
    pub fn new(max: u32) -> Self {
        Self { max, used: 0 }
    }

    /// Consume one iteration.
    ///
    /// # Errors
    ///
    /// Returns [`TerminalReason::IterationLimitReached`] when the budget is
    /// already exhausted; in that case `used` is left unchanged.
    pub fn tick(&mut self) -> Result<(), TerminalReason> {
        if self.used >= self.max {
            return Err(TerminalReason::IterationLimitReached);
        }
        self.used += 1;
        Ok(())
    }

    /// How many iterations remain before the budget is exhausted.
    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.used)
    }

    /// How many iterations have been consumed so far.
    #[must_use]
    pub fn used(&self) -> u32 {
        self.used
    }
}

/// A token-spend cap acting as a second, independent stop condition.
///
/// Tracks accumulated input and output tokens against per-direction limits.
/// [`add`](Self::add) accepts a spend only while *both* running totals stay at
/// or below their caps; once either would be exceeded it refuses the spend and
/// reports [`TerminalReason::BudgetExhausted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CostBudget {
    limit_input: u32,
    limit_output: u32,
    acc_input: u32,
    acc_output: u32,
}

impl CostBudget {
    /// Create a budget with separate caps for input and output tokens.
    #[must_use]
    pub fn new(limit_input: u32, limit_output: u32) -> Self {
        Self {
            limit_input,
            limit_output,
            acc_input: 0,
            acc_output: 0,
        }
    }

    /// Record a spend of `input_tokens` / `output_tokens`.
    ///
    /// Callers that hold an LLM usage value pass its `input_tokens` and
    /// `output_tokens` fields directly; `tdw-core` intentionally has no
    /// dependency on any LLM crate.
    ///
    /// # Errors
    ///
    /// Returns [`TerminalReason::BudgetExhausted`] if applying the spend would
    /// push either running total past its cap; in that case neither total is
    /// modified.
    pub fn add(&mut self, input_tokens: u32, output_tokens: u32) -> Result<(), TerminalReason> {
        let next_input = self.acc_input.saturating_add(input_tokens);
        let next_output = self.acc_output.saturating_add(output_tokens);
        if next_input > self.limit_input || next_output > self.limit_output {
            return Err(TerminalReason::BudgetExhausted);
        }
        self.acc_input = next_input;
        self.acc_output = next_output;
        Ok(())
    }

    /// Accumulated input tokens.
    #[must_use]
    pub fn used_input(&self) -> u32 {
        self.acc_input
    }

    /// Accumulated output tokens.
    #[must_use]
    pub fn used_output(&self) -> u32 {
        self.acc_output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_reason_as_str_is_stable() {
        assert_eq!(TerminalReason::NaturalStop.as_str(), "natural_stop");
        assert_eq!(
            TerminalReason::IterationLimitReached.as_str(),
            "iteration_limit_reached"
        );
        assert_eq!(TerminalReason::Cancelled.as_str(), "cancelled");
        assert_eq!(TerminalReason::BudgetExhausted.as_str(), "budget_exhausted");
        assert_eq!(TerminalReason::FatalError.as_str(), "fatal_error");
    }

    #[test]
    fn turn_outcome_new_stores_fields() {
        let outcome = TurnOutcome::new(TerminalReason::NaturalStop, 7);
        assert_eq!(outcome.reason, TerminalReason::NaturalStop);
        assert_eq!(outcome.iterations, 7);
    }

    #[test]
    fn iteration_budget_ticks_until_exhausted() {
        let mut budget = IterationBudget::new(2);
        assert_eq!(budget.remaining(), 2);
        assert_eq!(budget.used(), 0);

        assert_eq!(budget.tick(), Ok(()));
        assert_eq!(budget.used(), 1);
        assert_eq!(budget.remaining(), 1);

        assert_eq!(budget.tick(), Ok(()));
        assert_eq!(budget.used(), 2);
        assert_eq!(budget.remaining(), 0);

        assert_eq!(budget.tick(), Err(TerminalReason::IterationLimitReached));
        // A refused tick does not advance the counter.
        assert_eq!(budget.used(), 2);
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn iteration_budget_zero_max_is_immediately_exhausted() {
        let mut budget = IterationBudget::new(0);
        assert_eq!(budget.remaining(), 0);
        assert_eq!(budget.tick(), Err(TerminalReason::IterationLimitReached));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn cost_budget_accumulates_until_either_cap_is_hit() {
        let mut budget = CostBudget::new(100, 50);

        assert_eq!(budget.add(40, 20), Ok(()));
        assert_eq!(budget.used_input(), 40);
        assert_eq!(budget.used_output(), 20);

        // Spending up to the exact caps is allowed.
        assert_eq!(budget.add(60, 30), Ok(()));
        assert_eq!(budget.used_input(), 100);
        assert_eq!(budget.used_output(), 50);

        // Any further spend on either direction is refused.
        assert_eq!(budget.add(1, 0), Err(TerminalReason::BudgetExhausted));
        assert_eq!(budget.add(0, 1), Err(TerminalReason::BudgetExhausted));
        // A refused spend leaves both totals untouched.
        assert_eq!(budget.used_input(), 100);
        assert_eq!(budget.used_output(), 50);
    }

    #[test]
    fn cost_budget_output_cap_is_independent_of_input_cap() {
        let mut budget = CostBudget::new(1_000, 10);
        assert_eq!(budget.add(0, 10), Ok(()));
        // Input has plenty left, but the output cap is reached.
        assert_eq!(budget.add(5, 1), Err(TerminalReason::BudgetExhausted));
        assert_eq!(budget.used_input(), 0);
        assert_eq!(budget.used_output(), 10);
    }

    #[test]
    fn cost_budget_saturating_add_refuses_without_overflow_panic() {
        // A spend whose arithmetic would overflow `u32` must be refused via the
        // cap check rather than panicking: `saturating_add` clamps to `u32::MAX`,
        // which still exceeds the finite cap below.
        let mut budget = CostBudget::new(100, 100);
        assert_eq!(budget.add(10, 10), Ok(()));
        assert_eq!(
            budget.add(u32::MAX, 0),
            Err(TerminalReason::BudgetExhausted)
        );
        assert_eq!(
            budget.add(0, u32::MAX),
            Err(TerminalReason::BudgetExhausted)
        );
        // The refused spends left the accumulators untouched.
        assert_eq!(budget.used_input(), 10);
        assert_eq!(budget.used_output(), 10);
    }

    #[test]
    fn cost_budget_at_max_caps_clamps_instead_of_wrapping() {
        // With caps at the type max, saturating arithmetic clamps so an extra
        // spend stays at the cap and is accepted rather than wrapping to a small
        // value and silently resetting the budget.
        let mut budget = CostBudget::new(u32::MAX, u32::MAX);
        assert_eq!(budget.add(u32::MAX, u32::MAX), Ok(()));
        assert_eq!(budget.add(1, 1), Ok(()));
        assert_eq!(budget.used_input(), u32::MAX);
        assert_eq!(budget.used_output(), u32::MAX);
    }
}
