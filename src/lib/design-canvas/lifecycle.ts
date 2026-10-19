/** Design and Preview reuse one native surface per task. Await close before reuse. */
const transitions = new Map<number, Promise<void>>();

export function enqueueDesignTransition(taskId: number, operation: () => Promise<void>): Promise<void> {
  const next = (transitions.get(taskId) ?? Promise.resolve()).catch(() => {}).then(operation);
  transitions.set(taskId, next);
  void next.catch(() => {}).finally(() => {
    if (transitions.get(taskId) === next) transitions.delete(taskId);
  });
  return next;
}

export function waitForDesignTransition(taskId: number): Promise<void> {
  return (transitions.get(taskId) ?? Promise.resolve()).catch(() => {});
}
