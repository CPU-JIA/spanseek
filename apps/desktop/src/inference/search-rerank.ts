import type { SearchEvidence } from '@pkms/contracts';

export type SearchResultOrder = 'fusion' | 'keyword' | 'semantic';

const lexicalRank = (item: SearchEvidence) => item.lexical_rank ?? Number.POSITIVE_INFINITY;
const semanticScore = (item: SearchEvidence) => item.cosine_similarity ?? Number.NEGATIVE_INFINITY;
const fusionScore = (item: SearchEvidence) => item.fusion_rank ?? Number.NEGATIVE_INFINITY;

function compareId(left: SearchEvidence, right: SearchEvidence): number {
  return left.id.localeCompare(right.id);
}

/**
 * Reorders only the candidates already returned by the current search. It does
 * not widen Top-K, change the scope, or pretend to be a second retrieval pass.
 */
export function rerankSearchResults(
  items: readonly SearchEvidence[],
  order: SearchResultOrder,
): SearchEvidence[] {
  return [...items].sort((left, right) => {
    if (order === 'keyword') {
      if ((left.lexical_rank !== null) !== (right.lexical_rank !== null))
        return left.lexical_rank !== null ? -1 : 1;
      const rank = lexicalRank(left) - lexicalRank(right);
      if (rank !== 0 && Number.isFinite(rank)) return rank;
      const fusion = fusionScore(right) - fusionScore(left);
      if (fusion !== 0 && Number.isFinite(fusion)) return fusion;
      const cosine = semanticScore(right) - semanticScore(left);
      if (cosine !== 0 && Number.isFinite(cosine)) return cosine;
      return compareId(left, right);
    }
    if (order === 'semantic') {
      if ((left.cosine_similarity !== null) !== (right.cosine_similarity !== null))
        return left.cosine_similarity !== null ? -1 : 1;
      const cosine = semanticScore(right) - semanticScore(left);
      if (cosine !== 0 && Number.isFinite(cosine)) return cosine;
      const fusion = fusionScore(right) - fusionScore(left);
      if (fusion !== 0 && Number.isFinite(fusion)) return fusion;
      const rank = lexicalRank(left) - lexicalRank(right);
      if (rank !== 0 && Number.isFinite(rank)) return rank;
      return compareId(left, right);
    }
    if ((left.fusion_rank !== null) !== (right.fusion_rank !== null))
      return left.fusion_rank !== null ? -1 : 1;
    const fusion = fusionScore(right) - fusionScore(left);
    if (fusion !== 0 && Number.isFinite(fusion)) return fusion;
    const rank = lexicalRank(left) - lexicalRank(right);
    if (rank !== 0 && Number.isFinite(rank)) return rank;
    const cosine = semanticScore(right) - semanticScore(left);
    if (cosine !== 0 && Number.isFinite(cosine)) return cosine;
    return compareId(left, right);
  });
}
