# GitHub Setup

The repository is GitHub-ready, but remote creation changes external state and needs
an explicit owner/repo/visibility decision.

When approved, run:

```powershell
.\scripts\github\create-private-repo.ps1 -Owner <owner> -Name FinX-Plattform
git push -u origin main
```

The default visibility should be private because the project target is personal and
not OSS.

## Local GitHub Assets

- `.github/workflows/ci.yml`
- `.github/workflows/nightly.yml`
- `.github/workflows/codeql.yml`
- `.github/dependabot.yml`
- `.github/pull_request_template.md`
- `.github/ISSUE_TEMPLATE/`
