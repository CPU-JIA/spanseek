import UniqueID from '@tiptap/extension-unique-id';
import { MIRROR } from './mirror-transactions.ts';

/** Copy/paste gets new occurrence IDs; mirrored transactions retain the origin's IDs. */
export const referenceOccurrenceIDs = UniqueID.extend({ name: 'referenceOccurrenceIDs' }).configure(
  {
    attributeName: 'occurrence_id',
    types: ['noteReference'],
    generateID: () => crypto.randomUUID(),
    filterTransaction: (transaction) => !transaction.getMeta(MIRROR),
  },
);
