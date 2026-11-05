import type { WorkspaceIdentity } from '@pkms/contracts';
import { draftKey } from './drafts.ts';
import type { DraftController } from './draft-controller.ts';

/** Owns controllers until a workspace transition has durably drained them. */
export class DraftRegistry {
  private entries = new Map<string, Promise<DraftController>>();
  private locked = false;

  get(identity: WorkspaceIdentity, noteId: string, open: () => Promise<DraftController>) {
    if (this.locked) return Promise.reject(new Error('正在切换工作区，请稍候。'));
    const key = draftKey(identity, noteId);
    let pending = this.entries.get(key);
    if (!pending) {
      pending = open().catch((error) => {
        if (this.entries.get(key) === pending) this.entries.delete(key);
        throw error;
      });
      this.entries.set(key, pending);
    }
    return pending;
  }

  async existing(identity: WorkspaceIdentity, noteId: string) {
    const pending = this.entries.get(draftKey(identity, noteId));
    if (!pending) return null;
    try {
      return await pending;
    } catch {
      return null;
    }
  }

  private async controllers() {
    // A failed open has no in-memory edits and must not poison unrelated saves.
    const results = await Promise.allSettled(this.entries.values());
    return results.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []));
  }

  async flush() {
    await Promise.all((await this.controllers()).map((controller) => controller.flush()));
  }

  async freezeAndFlush() {
    if (this.locked) throw new Error('工作区操作尚未完成。');
    this.locked = true;
    const resumptions: Array<() => void> = [];
    const controllers = await this.controllers();
    const resume = () => {
      for (const thaw of resumptions) thaw();
      this.locked = false;
    };
    try {
      for (const controller of controllers) resumptions.push(controller.freeze());
      await Promise.all(controllers.map((controller) => controller.flush()));
      return (release: boolean) => {
        try {
          if (release) {
            for (const controller of controllers) controller.release();
            this.entries.clear();
          }
        } finally {
          resume();
        }
      };
    } catch (error) {
      resume();
      throw error;
    }
  }
}
