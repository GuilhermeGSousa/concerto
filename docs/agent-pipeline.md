# The agent pipeline

Claude Code sessions take issues labelled `agent`, open one pull request per
issue, and carry it through implementation and review. An issue labelled
`agent:spec` goes through questions and a spec first, and you approve the spec
in the PR; the agent then plans and implements without waiting on you. You merge. The procedure the agents follow is
[.agents/skills/issue-pipeline/SKILL.md](../.agents/skills/issue-pipeline/SKILL.md).

## Handing over an issue

Add `agent` to an issue you opened. Add as well:

| Label | Effect |
|---|---|
| `agent:spec` | Questions and a spec approved by you in the PR, then a plan the agent follows without approval. For large tasks that change foundational work. Without it, the agent goes straight to an implementation PR. |
| `agent:local` | Only the runner on your machine takes it. Use it for rendering, UI and editor work, which needs a look at the running program. |
| `agent:priority` | Runners take it, or its blockers while it is blocked, before issues without it. |
| `documentation` or `lint` | Sorting only; no effect on the path. |

An agent adds `agent:spec` itself, and says why in the PR, only when the issue
turns out to be a large task that changes foundational work: the ECS,
scheduling, the frame loop, the render architecture, the asset pipeline, or a
public API that several crates or games build on. Any other design decision it
asks you in a PR comment, and it waits for your answer. Remove the label to send the issue
back to the short path. A cloud agent sees a label change only at its next
wake, so comment on the PR after changing one.

Agents also file issues for work they find outside their task, and for
follow-ups agreed in a session, without asking. Those carry `agent:filed`, and
`agent:spec` when they are large and foundational, and enter the queue like
yours. Remove
`agent` from one to keep agents off it.

An issue whose body has a line `Blocked by #a, #b` is not taken while any of
those issues is open. Runners take `agent:priority` issues first, then the
rest, each oldest first. A blocked `agent:priority` issue sends them to its
blockers first, and to theirs when those are blocked too, whether or not the
blockers carry `agent:priority`. A blocker without `agent`, opened by someone
else, already claimed, or labelled `agent:local` for the cloud runner is
passed over.

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
   If the issue turns out to be large and foundational, the agent adds
   `agent:spec` and switches to the path below; any other design decision it
   asks you in a comment.
2. **Review**, as below.

An issue with `agent:spec` goes through:

1. **Questions**, one comment with every open decision. Answer in comments.
2. **Spec**, written into the PR description. Comment `/approve`, alone on its
   line, or give feedback.
3. **Plan**, written into the PR description once the spec is approved. The
   agent does not wait for approval; comment on it if you want it changed.
4. **Implementation**, one commit or more per plan task.
5. **Review**: once CI is green, `branch-reviewer` runs and its report is
   posted. The PR is then marked ready for review.

Merge the PR yourself; agents never do. Comment `/stop`, or close the PR, to
stop the agent. It removes `agent` from the issue and leaves the branch. Add
`agent` again to hand the issue back; the next agent resumes from the branch.

Cloud agents run only `cargo fmt` and the test budget check, plus
`cargo metadata` to update `Cargo.lock` when a `Cargo.toml` changed, and leave clippy, tests and the wasm build to CI.
For rendering, UI and editor changes they list the visual checks left for you
in the review comment.

When the repository is out of Actions minutes, GitHub fails each CI job without
starting it. Agents do not treat that as a failure and do not wait for it: they
review and mark the PR ready as usual, and the review comment names the jobs
that did not run and the done gate commands nobody has run on the head commit.
Run those yourself, or re-run CI once there are minutes, before you merge. The
runner on your machine runs the full done gate itself, so its PRs lose nothing.

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
if it holds fewer than 2, preferring `agent:local` ones within each priority tier. It works in worktrees
under `../concerto-issues/`, sharing one `target/` there, and never touches your
checkout. When it moves to a different worktree than the last one built, it runs
`cargo clean --workspace` there first. Worktrees otherwise reuse each other's
builds of the workspace crates, so the first build after a switch rebuilds them,
Jolt included. Third-party crates stay shared. It runs the full done gate and
checks visual changes on screen. When the disk holding the shared `target/` has
under 50 GB free, it deletes the
incremental data there, then the whole `target/` if that wasn't enough and it
has nothing running, and it takes no new issue until there's room. If the disk
is still short with `target/` gone, it sends you a notification and stops that
tick. It stops when you close the session; its issues resume at the next start.

The roles and skills are reached through the `.claude/agents` and
`.claude/skills` symlinks. On Windows they need `git config core.symlinks true`
and Developer Mode, set before cloning.
