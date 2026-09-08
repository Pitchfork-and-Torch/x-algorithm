"""Safemodel sex-and-nudity scores are comparison-only.

The classifier returns MPAA-style R/X buckets. R is not a policy hit.
NsfwHighRecall / NsfwHighPrecision drops are OON For You kills and must
come from a PTOS AdultContentSexualHard decision, not from safemodel
alone.
"""


def should_apply_safemodel_nsfw_drop_labels(
    safemodel_positive: bool, ptos_confirmed_hard_nsfw: bool
) -> bool:
    return bool(safemodel_positive and ptos_confirmed_hard_nsfw)
