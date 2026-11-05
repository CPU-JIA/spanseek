import { convertSourceNote } from './source-note-conversion';

self.onmessage = (event) => {
  try {
    self.postMessage({ value: convertSourceNote(event.data.text, event.data.format) });
  } catch {
    self.postMessage({ error: 'conversion_failed' });
  }
};
