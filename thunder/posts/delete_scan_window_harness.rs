//! Standalone proof that mark_as_deleted must strip author deques.
//! thunder/ has no Cargo.toml in this snapshot, so crate tests cannot run.
//!
//!   rustc --edition 2021 thunder/posts/delete_scan_window_harness.rs -o /tmp/delete_scan_window_harness
//!   /tmp/delete_scan_window_harness

use std::collections::{HashMap, VecDeque};

const MAX_TINY_POSTS_PER_USER_SCAN: usize = 500;
const MAX_ORIGINAL_POSTS_PER_AUTHOR: usize = 50;

#[derive(Clone)]
struct TinyPost {
    post_id: i64,
}

struct Store {
    posts: HashMap<i64, i64>,
    original: HashMap<i64, VecDeque<TinyPost>>,
}

impl Store {
    fn insert(&mut self, post_id: i64, author_id: i64) {
        self.posts.insert(post_id, author_id);
        self.original
            .entry(author_id)
            .or_default()
            .push_back(TinyPost { post_id });
    }

    fn mark_as_deleted_buggy(&mut self, post_id: i64) {
        self.posts.remove(&post_id);
    }

    fn mark_as_deleted_fixed(&mut self, post_id: i64) {
        if let Some(author_id) = self.posts.remove(&post_id) {
            if let Some(deque) = self.original.get_mut(&author_id) {
                deque.retain(|tiny| tiny.post_id != post_id);
            }
        }
    }

    fn get_all(&self, author_id: i64) -> Vec<i64> {
        let Some(deque) = self.original.get(&author_id) else {
            return Vec::new();
        };
        deque
            .iter()
            .rev()
            .take(MAX_TINY_POSTS_PER_USER_SCAN)
            .filter_map(|tiny| self.posts.get(&tiny.post_id).map(|_| tiny.post_id))
            .take(MAX_ORIGINAL_POSTS_PER_AUTHOR)
            .collect()
    }
}

fn seed() -> Store {
    let mut store = Store {
        posts: HashMap::new(),
        original: HashMap::new(),
    };
    for id in 1..=600 {
        store.insert(id, 42);
    }
    store
}

fn main() {
    let mut buggy = seed();
    for id in 101..=600 {
        buggy.mark_as_deleted_buggy(id);
    }
    let buggy_hits = buggy.get_all(42);
    assert!(
        buggy_hits.is_empty(),
        "hypothesis: newest 500 deleted ids eat the scan window, got {buggy_hits:?}"
    );
    assert_eq!(buggy.original.get(&42).map(|d| d.len()), Some(600));
    assert_eq!(buggy.posts.len(), 100);

    let mut fixed = seed();
    for id in 101..=600 {
        fixed.mark_as_deleted_fixed(id);
    }
    let fixed_hits = fixed.get_all(42);
    assert_eq!(fixed_hits.len(), MAX_ORIGINAL_POSTS_PER_AUTHOR);
    assert_eq!(fixed_hits, (51..=100).rev().collect::<Vec<_>>());
    assert_eq!(fixed.original.get(&42).map(|d| d.len()), Some(100));
    assert_eq!(fixed.posts.len(), 100);

    println!("ok: buggy returns []; fixed returns 50 live ids from the remaining 100");
}
