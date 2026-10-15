//! In-memory BK-tree metric index for 64-bit Hamming distances.

use dbsrt_core::hamming_distance;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BkItem {
    pub phash: u64,
    pub offset: usize,
}

pub struct BkTree {
    root: Option<BkTreeNode>,
    size: usize,
}

struct BkTreeNode {
    item: BkItem,
    children: HashMap<u32, BkTreeNode>,
}

impl BkTreeNode {
    fn new(item: BkItem) -> Self {
        Self {
            item,
            children: HashMap::new(),
        }
    }

    fn insert(&mut self, item: BkItem) {
        let dist = hamming_distance(self.item.phash, item.phash);
        if dist == 0 && self.item.offset == item.offset {
            return;
        }
        match self.children.get_mut(&dist) {
            Some(child) => child.insert(item),
            None => {
                self.children.insert(dist, BkTreeNode::new(item));
            }
        }
    }

    fn search(&self, target: u64, max_distance: u32, results: &mut Vec<BkItem>) {
        let dist = hamming_distance(self.item.phash, target);
        if dist <= max_distance {
            results.push(self.item);
        }

        let low = dist.saturating_sub(max_distance);
        let high = dist + max_distance;

        for (&edge_dist, child) in &self.children {
            if edge_dist >= low && edge_dist <= high {
                child.search(target, max_distance, results);
            }
        }
    }
}

impl Default for BkTree {
    fn default() -> Self {
        Self::new()
    }
}

impl BkTree {
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
        }
    }

    pub fn insert(&mut self, item: BkItem) {
        match &mut self.root {
            Some(root) => root.insert(item),
            None => self.root = Some(BkTreeNode::new(item)),
        }
        self.size += 1;
    }

    pub fn search(&self, target: u64, max_distance: u32) -> Vec<BkItem> {
        let mut results = Vec::new();
        if let Some(root) = &self.root {
            root.search(target, max_distance, &mut results);
        }
        results
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bktree_insert_search() {
        let mut tree = BkTree::new();
        tree.insert(BkItem {
            phash: 0b0000,
            offset: 0,
        });
        tree.insert(BkItem {
            phash: 0b0001,
            offset: 1,
        });
        tree.insert(BkItem {
            phash: 0b0011,
            offset: 2,
        });
        tree.insert(BkItem {
            phash: 0b1111,
            offset: 3,
        });

        let res = tree.search(0b0000, 1);
        assert_eq!(res.len(), 2);

        let res_exact = tree.search(0b0011, 0);
        assert_eq!(res_exact.len(), 1);
        assert_eq!(res_exact[0].offset, 2);
    }
}
