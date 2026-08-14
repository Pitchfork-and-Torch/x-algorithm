# Feed fairness: merit over reach

This package makes For You rank by predicted value for the viewer, not by how large an author's existing audience already is — while keeping user mutes/blocks consistent and stopping viral originals from flooding a slate through many retweeters.

It is **not** demographic parity, identity quotas, or equal impressions for every account. Low Phoenix quality stays low. Spam and predicted block/mute/report stay negative and are not lifted.

## Changes

### 1. Author-size IPS (ranking residual)

Equal Phoenix scores should not be ordered by follower count. Horvitz–Thompson residual on `ln(1 + followers)`, mean-normalized inside the scored batch.

Full math, knobs, and worked example: [`MERITOCRATIC_AUTHOR_SIZE_IPS.md`](MERITOCRATIC_AUTHOR_SIZE_IPS.md).

Code: `home-mixer/scorers/author_size_ips.rs`.

### 2. Size-aware out-of-network relief

Discovery for accounts a viewer does not follow is almost entirely OON. The flat `OonWeightFactor` (default 0.75) stacked on audience-size leakage double-penalizes small creators.

```
oon' = base + (1 - base) * relief * t
```

`t` is 1 at or below `SizeAwareOonFollowerFloor` (default 1k), 0 at or above `SizeAwareOonFollowerCeiling` (default 100k), linear between. Default `relief = 0.5` → a 500-follower OON post uses 0.875 instead of 0.75. Missing follower counts keep the base tax. Large OON accounts keep the full tax.

| Param | Default | Meaning |
|---|---|---|
| `rust_home_mixer_enable_size_aware_oon_relief` | true | Master switch |
| `rust_home_mixer_size_aware_oon_follower_floor` | 1000 | Full relief at/below |
| `rust_home_mixer_size_aware_oon_follower_ceiling` | 100000 | No relief at/above |
| `rust_home_mixer_size_aware_oon_relief` | 0.5 | Share of the gap to 1.0 |

Code: `RankingScorer::oon_weight_for` in `home-mixer/scorers/ranking_scorer.rs`.

### 3. Origin-author diversity

Author diversity used `candidate.author_id`. A viral original could occupy many slate slots via distinct retweeters, each counted as a first appearance.

With `EnableOriginAuthorDiversity` (default true), diversity counts `get_original_author_id()` (`retweeted_user_id` when present). The second and later appearances of the same original author decay.

Quotes still diversity-key on the quoter: a quote is new content by that author.

| Param | Default | Meaning |
|---|---|---|
| `rust_home_mixer_enable_origin_author_diversity` | true | Decay on content origin for retweets |

### 4. Mute/block symmetry for quotes and reposts

`AuthorSocialgraphFilter` already dropped quotes and reposts of **blocked** authors. Mute only checked the candidate author, so a quote or repost of a muted user could still serve.

Mute now mirrors block for `quoted_user_id` and `retweeted_user_id`. Viewer preference is respected whether the muted account posts directly or is amplified by someone else.

Code: `home-mixer/filters/author_socialgraph_filter.rs`.

## What this does not do

- Does not replace retrieval. A post that never enters the candidate set cannot be residualized. SidTail / `post_creation` retrieval remain the natural next step.
- Does not use UserCred / PageRank as a For You weight.
- Does not soften safety or spam drops. Visibility filtering stays separate from ranking.

## References

- Ashudeep Singh and Thorsten Joachims. Fairness of Exposure in Rankings. KDD 2018. https://arxiv.org/abs/1802.07281
- Asia J. Biega, Krishna P. Gummadi, and Gerhard Weikum. Equity of Attention: Amortizing Individual Fairness in Rankings. SIGIR 2018. https://arxiv.org/abs/1805.01788
