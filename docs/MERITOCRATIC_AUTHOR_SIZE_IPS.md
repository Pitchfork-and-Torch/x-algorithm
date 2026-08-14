# Meritocratic author-size IPS

This is a ranking residual, not a quota.

Phoenix predicts `P(action | viewer, post)` per impression. That is the quality signal. Author follower count is not a Phoenix feature. Audience size still leaks into For You through the follow graph (Thunder), hashed author IDs, SimClusters log-favorite retrieval, and the flat 0.75 out-of-network tax. Two posts with the same predicted value for this viewer should not be ordered by how many people already follow the author.

## What this is

Singh and Joachims (KDD 2018), Example 3: speakers get access to willing listeners in proportion to relevance, not in proportion to prior reach. Biega, Gummadi, and Weikum (SIGIR 2018) call the same idea equity of attention: exposure tracks merit.

Take groups of size one (every author is their own group). Disparate treatment says:

```
exposure(i) / merit(i)  ~=  exposure(j) / merit(j)
```

Merit here is the Phoenix weighted score. Historical show probability grows with `ln(1 + followers)`. The Horvitz-Thompson correction is `1 / ln(1 + followers)`, then mean-normalized inside the scored batch so the adjustment reallocates score instead of inflating it:

```
p_i   = ln(1 + max(followers_i, 1))
ips_i = 1 / p_i
m_i   = 1 + alpha * (ips_i / mean(ips) - 1)
m_i   = clamp(m_i, 1 / max_boost, max_boost)
score'_i = score_i * m_i     if score_i > 0
         = score_i           otherwise
```

Missing follower counts are identity (`m_i = 1`). Negative scores are not lifted. `alpha = 0` is a no-op. Default `alpha = 0.5`, `max_boost = 2.0`.

Worked example at the defaults, authors with 100 / 10k / 1M followers:

```
p     ~= 4.62 / 9.21 / 13.82
ips   ~= 0.217 / 0.109 / 0.072
m     ~= 1.32 / 0.91 / 0.77
```

A 100-follower post and a 1M-follower post that Phoenix scored equally (pairwise) rank 1.25 / 0.75 = 1.67 apart. A 1M-follower post that Phoenix scored 3x higher still wins (3 * 0.75 > 1 * 1.25).

## What this is not

- Not demographic parity. There are no identity groups, no protected-class buckets, no guaranteed impression share by category.
- Not "every account gets the same impressions." Low-quality posts stay low. Spam and predicted block/mute/report stay negative and are not boosted.
- Not a replacement for retrieval. A post that never enters the candidate set cannot be residualized. SidTail (authors under 1k followers) and a SID `post_creation` window are the retrieval-side counterparts. They are not wired in this change. Ranking-side companions (size-aware OON relief, origin-author diversity, mute symmetry) are in [`FEED_FAIRNESS.md`](FEED_FAIRNESS.md).
- Not UserCred. PageRank is an enforcement prestige graph, not a For You weight. Do not add it as a rank feature.

## Knobs

| Param | Default | Meaning |
|---|---|---|
| `rust_home_mixer_enable_author_size_ips` | true | Master switch |
| `rust_home_mixer_author_size_ips_alpha` | 0.5 | 0 = off, 1 = full batch IPS |
| `rust_home_mixer_author_size_ips_max_boost` | 2.0 | Clamp on the multiplier |

Code: `home-mixer/scorers/author_size_ips.rs`, applied in `RankingScorer` after the Phoenix weighted sum and before author diversity, the OON tax, and cold start.

## References

- Ashudeep Singh and Thorsten Joachims. Fairness of Exposure in Rankings. KDD 2018. https://arxiv.org/abs/1802.07281
- Asia J. Biega, Krishna P. Gummadi, and Gerhard Weikum. Equity of Attention: Amortizing Individual Fairness in Rankings. SIGIR 2018. https://arxiv.org/abs/1805.01788
- D. G. Horvitz and D. J. Thompson. A Generalization of Sampling Without Replacement From a Finite Universe. JASA 1952.
