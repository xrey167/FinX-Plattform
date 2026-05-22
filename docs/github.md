# GitHub Setup

The repository is live at
[github.com/xrey167/FinX-Plattform](https://github.com/xrey167/FinX-Plattform).
Visibility is **public** — chosen so GitHub Actions and branch protection
rulesets are available on the Free tier (both are gated behind GitHub Pro for
private repos). Any change to the remote owner, name, or visibility requires a
new explicit approval per `AGENTS.md`.

To reproduce the bootstrap, run:

```powershell
.\scripts\github\create-private-repo.ps1 -Owner <owner> -Name FinX-Plattform -Visibility public
git push -u origin main
```

(The script name says "private" for historical reasons; the `-Visibility`
parameter is the source of truth.)

## Local GitHub Assets

- `.github/workflows/ci.yml`
- `.github/workflows/nightly.yml`
- `.github/workflows/codeql.yml`
- `.github/dependabot.yml`
- `.github/pull_request_template.md`
- `.github/ISSUE_TEMPLATE/`
