# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 X.AI Corp.
"""Topic retrieval must not fail-open on production parent topic IDs."""

from xrex.models.topic_categories import (
    TOPIC_ID_TO_BITS,
    _POST_SIDE_GROUP_MEMBERS,
    build_request_topic_bitmasks,
    request_topic_ids_to_bitmap,
    topic_ids_to_bitmap,
)

# grok_topics.rs / TopicIdExpansion production parent IDs.
TOPIC_SPORTS = 1925949452197478400
TOPIC_TECHNOLOGY = 1925949722688126976
TOPIC_SCIENCE = 1925949744683114496
TOPIC_ATHLETICS = 1925950838414942210
TOPIC_MUSIC = 1925949604224196608
SPORTS_GROUP = 1000000000000000004
SCIENCE_TECHNOLOGY_GROUP = 1000000000000000001


def test_parent_topic_ids_are_only_on_the_post_side_map():
    for tid in (TOPIC_SPORTS, TOPIC_SCIENCE, TOPIC_TECHNOLOGY, TOPIC_ATHLETICS):
        assert tid not in TOPIC_ID_TO_BITS
        assert tid in _POST_SIDE_GROUP_MEMBERS


def test_request_mask_is_nonzero_for_sports_science_technology():
    for tid in (TOPIC_SPORTS, TOPIC_SCIENCE, TOPIC_TECHNOLOGY, TOPIC_ATHLETICS):
        mask = request_topic_ids_to_bitmap([tid])
        assert mask != 0, f"{tid} must not fail-open as a zero user mask"


def test_request_mask_matches_post_mask_for_parent_ids():
    for tid in (TOPIC_SPORTS, TOPIC_SCIENCE, TOPIC_TECHNOLOGY, TOPIC_ATHLETICS):
        assert request_topic_ids_to_bitmap([tid]) == topic_ids_to_bitmap([tid])


def test_sports_request_overlaps_sports_post_not_science():
    sports_req = request_topic_ids_to_bitmap([TOPIC_SPORTS])
    sports_post = topic_ids_to_bitmap([TOPIC_SPORTS])
    science_post = topic_ids_to_bitmap([TOPIC_SCIENCE])
    assert sports_req & sports_post
    assert not (sports_req & science_post)


def test_science_request_overlaps_science_and_technology_group():
    science_req = request_topic_ids_to_bitmap([TOPIC_SCIENCE])
    tech_req = request_topic_ids_to_bitmap([TOPIC_TECHNOLOGY])
    group_post = topic_ids_to_bitmap([SCIENCE_TECHNOLOGY_GROUP])
    assert science_req & group_post
    assert tech_req & group_post
    assert science_req & tech_req


def test_empty_or_zero_ids_stay_passthrough():
    assert request_topic_ids_to_bitmap([]) == 0
    assert request_topic_ids_to_bitmap(None) == 0
    assert request_topic_ids_to_bitmap([0]) == 0
    assert request_topic_ids_to_bitmap([0, 0]) == 0


def test_known_leaf_and_group_ids_still_set_bits():
    assert request_topic_ids_to_bitmap([TOPIC_MUSIC]) != 0
    assert request_topic_ids_to_bitmap([SPORTS_GROUP]) != 0
    assert request_topic_ids_to_bitmap([SPORTS_GROUP]) & topic_ids_to_bitmap([TOPIC_SPORTS])


def test_build_request_topic_bitmasks_pads_and_truncates():
    padded = build_request_topic_bitmasks([[TOPIC_SPORTS]], 3)
    assert len(padded) == 3
    assert padded[0] == request_topic_ids_to_bitmap([TOPIC_SPORTS])
    assert padded[1] == 0
    assert padded[2] == 0

    truncated = build_request_topic_bitmasks(
        [[TOPIC_SPORTS], [TOPIC_SCIENCE], [TOPIC_MUSIC]], 1
    )
    assert truncated == [request_topic_ids_to_bitmap([TOPIC_SPORTS])]


def test_build_request_topic_bitmasks_empty_request_is_passthrough():
    assert build_request_topic_bitmasks(None, 2) == [0, 0]
    assert build_request_topic_bitmasks([], 2) == [0, 0]
    assert build_request_topic_bitmasks([[], [0]], 2) == [0, 0]


def test_topic_ids_only_lookup_would_fail_open_on_parents():
    """Document the old serving path: TOPIC_ID_TO_BITS-only → mask 0 → leak organic."""
    for tid in (TOPIC_SPORTS, TOPIC_SCIENCE, TOPIC_TECHNOLOGY, TOPIC_ATHLETICS):
        old_mask = 0
        if tid in TOPIC_ID_TO_BITS:
            for bit in TOPIC_ID_TO_BITS[tid]:
                old_mask |= 1 << bit
        assert old_mask == 0
        assert request_topic_ids_to_bitmap([tid]) != 0
