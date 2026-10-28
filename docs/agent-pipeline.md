# The agent pipeline

Claude Code sessions take issues labelled `agent`, open one pull request per
issue, and carry it through implementation and review. An issue labelled
`agent:spec` goes through questions, spec and plan first, and you approve each
in the PR. You merge. The procedure the agents follow is
[.agents/skills/issue-pipeline/SKILL.md](../.agents/skills/issue-pipeline/SKILL.md).

## Handing over an issue

Add `agent` to an issue you opened. Add as well:

| Label | Effect |
|---|---|
| `agent:spec` | Questions, spec and plan, each approved by you in the PR. Without it, the agent goes straight to an implementation PR. |
| `agent:local` | Only the runner on your machine takes it. Use it for rendering, UI and editor work, which needs a look at the running program. |
| `documentation` or `lint` | Sorting only; no effect on the path. |

An agent that meets a design decision on an issue without `agent:spec` adds
the label itself and says why in the PR. Remove the label to send the issue
back to the short path. A cloud agent sees a label change only at its next
wake, so comment on the PR after changing one.

Agents also file issues for work they find outside their task, and for
follow-ups agreed in a session, without asking. Those carry `agent:filed`, and
`agent:spec` when they need design, and enter the queue like yours. Remove
`agent` from one to keep agents off it.

The pipeline sets `agent:claimed-cloud` or `agent:claimed-local` on an issue it
has taken, and `agent:needs-owner` on a PR that waits on you.

## In the PR

Agents act through your GitHub account, so their comments show you as the
author. Every one starts with a hidden `<!-- concerto-agent -->` line. An
`@` mention from an agent does not notify you; instead, when a PR needs spec
discussion you get a push notification in the Claude app, and every PR that
waits on you carries `agent:needs-owner`.

An issue without `agent:spec` goes through:

1. **Implementation**, test-first where code changes. The issue is the spec.
   If the agent meets a design decision, it adds `agent:spec` and switches to
   the path below.
2. **Review**, as below.

An issue with `agent:spec` goes through:

1. **Questions**, one comment with every open decision. Answer in comments.
2. **Spec**, written into the PR description. Comment `/approve`, alone on its
   line, or give feedback.
3. **Plan**, written into the PR description. `/approve` again.
4. **Implementation**, one commit or more per plan task.
5. **Review**: once CI is green, `branch-reviewer` runs and its report is
   posted. The PR is then marked ready for review.

Merge the PR yourself; agents never do. Comment `/stop`, or close the PR, to
stop the agent. It removes `agent` from the issue and leaves the branch. Add
`agent` again to hand the issue back; the next agent resumes from the branch.

Cloud agents run only `cargo fmt` and leave clippy, tests and the wasm build to
CI. For rendering, UI and editor changes they list the visual checks left for
you in the review comment.

## Runners

**Cloud.** An hourly Claude Code scheduled task takes at most one new issue per
run and holds at most 3. Each issue's agent wakes on your comments, reviews and
CI results. To pause it, disable the scheduled task. Do not delete it while a PR
is open: deleting it deletes the sessions driving those PRs.

**Your machine.** In your checkout run

```sh
claude remote-control
```

and in that session `/loop 1h /issue-pipeline`. You follow it from the Claude
app. Each hour it moves its PRs forward from your comments and takes a new issue
if it holds fewer than 2, preferring `agent:local` ones. It works in worktrees
under `../concerto-issues/`, sharing one `target/` there, and never touches your
checkout. It runs the full done gate and checks visual changes on screen. It
stops when you close the session; its issues resume at the next start.

The roles and skills are reached through the `.claude/agents` and
`.claude/skills` symlinks. On Windows they need `git config core.symlinks true`
and Developer Mode, set before cloning.
