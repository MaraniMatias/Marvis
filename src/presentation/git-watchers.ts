import { reactive, toValue, watch } from "vue";
import type { MaybeRefOrGetter } from "vue";
import type { Repo } from "../domain/workspace";
import { unwatchGitRepo, watchGitRepo } from "../lib/ipc";

/**
 * Keeps a watcher on every Git repository in the workspace.
 *
 * The sidebar names every checkout at once, so the numbers on the rows nobody is looking at
 * have to be as fresh as the one that is. That only holds if every checkout has a change
 * signal: an unwatched worktree is one the backend cannot tell to re-read, so its row would
 * describe a change set that had already moved on. Watching the repository rather than the
 * checkout is what makes the signal complete, because the worktrees of a repo share the Git
 * directory a commit is published through.
 *
 * Returns the repositories whose watcher could not start. Nothing can refresh the rows of a
 * checkout whose repository is in it, so the panel that shows those rows is where the reason
 * belongs: a toast expires and leaves the numbers stale with nothing to explain them.
 */
export function useGitWatchers(repos: MaybeRefOrGetter<Repo[]>): ReadonlySet<string> {
  const unwatched = reactive(new Set<string>());
  /** Each watched repository and the worktrees it was watched for, so a worktree joining or
   *  leaving one that is already watched re-registers it rather than going unwatched. */
  const watched = new Map<string, string>();
  const starting = new Set<string>();

  /** Registration is one call at a time per repository: a burst of workspace edits would
   *  otherwise start several watches of the same repo, and the backend keeps the first. */
  function register(repoId: string, worktrees: string): void {
    if (watched.get(repoId) === worktrees || starting.has(repoId)) return;
    watched.set(repoId, worktrees);
    starting.add(repoId);
    void watchGitRepo(repoId)
      .then(() => void unwatched.delete(repoId))
      .catch(() => {
        // The next change asks again: a repository that could not be watched once may be
        // watchable later, and a row that never refreshes is worse than a retried call.
        watched.delete(repoId);
        unwatched.add(repoId);
      })
      .finally(() => starting.delete(repoId));
  }

  function release(repoId: string): void {
    watched.delete(repoId);
    unwatched.delete(repoId);
    void unwatchGitRepo(repoId).catch(() => undefined);
  }

  watch(
    () => {
      const wanted = new Map<string, string>();
      for (const repo of toValue(repos)) {
        if (repo.kind !== "git") continue;
        const live = repo.checkouts
          .filter((checkout) => !checkout.isMissing)
          .map((checkout) => checkout.id)
          .sort();
        if (live.length > 0) wanted.set(repo.id, live.join(","));
      }
      return wanted;
    },
    (wanted) => {
      for (const repoId of [...watched.keys()]) {
        if (!wanted.has(repoId)) release(repoId);
      }
      for (const [repoId, worktrees] of wanted) register(repoId, worktrees);
    },
    { immediate: true },
  );

  return unwatched;
}
