# GitHub Setup

The repository is live at
[github.com/xrey167/FinX-Plattform](https://github.com/xrey167/FinX-Plattform).
Visibility is **public** — chosen so GitHub Actions and branch protection
rulesets are available on the Free tier (both are gated behind GitHub Pro for
private repos). Any change to the remote owner, name, or visibility requires a
new explicit approval per `AGENTS.md`.

To reproduce the bootstrap (e.g., for a fork), run from the repo root:

```powershell
gh repo create <owner>/FinX-Plattform --public --source . --remote origin --description "Rust workspace for the FinX-Finance trading data warehouse"
git push -u origin main
```

The repo is intentionally public from the outset — see the visibility note
above. No wrapper script is kept in-tree; the `gh` CLI is the source of
truth.

## Local GitHub Assets

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/nightly.yml`
- `.github/workflows/codeql.yml`
- `.github/dependabot.yml`
- `.github/pull_request_template.md`
- `.github/ISSUE_TEMPLATE/`
