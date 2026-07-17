# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. For multi-line bodies, prefer `--body-file <path>`.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`.
- **Apply or remove labels**: `gh issue edit <number> --add-label "..."` or `--remove-label "..."`.
- **Close**: `gh issue close <number> --comment "..."`.

Infer the repo from `git remote -v`; `gh` does this automatically inside the clone.

## Pull requests as a triage surface

**PRs as a request surface: no.**

GitHub shares one number space across issues and PRs, so a bare issue number may be either. Resolve ambiguity with `gh pr view <number>` and fall back to `gh issue view <number>`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: Create one issue labelled `wayfinder:map`, holding Destination, Notes, Decisions-so-far, Not-yet-specified, and Out-of-scope sections.
- **Child ticket**: Link an issue to the map as a GitHub sub-issue through the sub-issues API. Where sub-issues are unavailable, add the child to a task list in the map body and put `Part of #<map>` at the top of the child's body. Apply one of `wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling`, or `wayfinder:task`.
- **Blocking**: Use GitHub's native issue dependencies. Add an edge with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where the blocker database id comes from `gh api repos/<owner>/<repo>/issues/<number> --jq .id`. If native dependencies are unavailable, add `Blocked by: #<number>` to the child body.
- **Frontier query**: List the map's open children, excluding tickets with open blockers or an assignee. The first remaining child in map order wins.
- **Claim**: `gh issue edit <number> --add-assignee @me`; this is the session's first write.
- **Resolve**: Comment with the answer, close the ticket, then append a short linked context pointer to the map's Decisions-so-far section.
