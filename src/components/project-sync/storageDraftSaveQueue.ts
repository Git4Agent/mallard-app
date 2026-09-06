export class StorageDraftSaveQueue {
  private tail: Promise<void> = Promise.resolve();

  enqueue<T>(save: () => Promise<T>): Promise<T> {
    const result = this.tail.then(save);
    this.tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  beforeNavigation<T>(save: () => Promise<T>, navigate: () => void): Promise<T> {
    return this.enqueue(async () => {
      const result = await save();
      navigate();
      return result;
    });
  }
}
