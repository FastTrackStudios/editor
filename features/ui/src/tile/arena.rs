//! The tile arena.
//!
//! All tiles for one editor view live in a single `Vec<Tile>`;
//! cross-tile references are `TileId` indexes. This is the
//! Rust-idiomatic alternative to CM6's `tile.parent` /
//! `tile.children` direct references.
//!
//! ## Why arena
//!
//! - No `Rc<RefCell<...>>`. Every borrow is checked at compile
//!   time once we have `&Arena` or `&mut Arena`.
//! - No fragmented allocation per tile — one growable Vec.
//! - Parent/child cycles in CM6's pointer graph become plain
//!   integers that the borrow checker is happy with.
//! - Identity is stable across moves: a `TileId` survives the
//!   arena growing or compacting (within the same generation).
//!
//! ## What's missing in v1
//!
//! - **Generation counters / slot recycling.** A removed
//!   tile's slot is just left as `None`. We'll add
//!   slot reuse + generation when memory becomes a
//!   concern; for now, editor docs are small and the
//!   arena resets per view.
//! - **Bulk destroy.** CM6's `destroy()` chains; we'll add a
//!   recursive remove once the tree mutators land.

use crate::tile::flag::TileFlagSet;
use crate::tile::{Tile, TileBody, TileKind};

/// Stable handle to a tile in the arena. `u32` is plenty —
/// a doc with more than 4 billion tiles is not on the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId(pub u32);

impl TileId {
    #[must_use]
    pub fn as_usize(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// Slab of tiles. Each slot is either occupied or vacant
/// (after a future `remove`).
#[derive(Debug, Clone)]
pub struct Arena {
    /// Tiles. `None` is a vacant slot; we don't reuse yet.
    tiles: Vec<Option<Tile>>,
    /// Handed out by [`Arena::get`]/[`Arena::get_mut`] when an id names a
    /// vacant slot.
    ///
    /// Reaching for a destroyed tile is a caller bug, and this used to assert
    /// it. But `get` backs the `Index`/`IndexMut` impls that the whole tile
    /// module reads through, so asserting means one stale id anywhere takes
    /// down the application embedding the editor mid-render. An inert tile —
    /// no parent, no children, zero length — makes that same bug render as
    /// nothing at all, which is both survivable and visible.
    ///
    /// Writes through `get_mut` land here and are never read back as a real
    /// tile; the slot exists to satisfy the borrow, not to store anything.
    dead: Tile,
}

impl Default for Arena {
    fn default() -> Self {
        Self {
            tiles: Vec::new(),
            dead: Tile {
                parent: None,
                children: Vec::new(),
                length: 0,
                kind: TileKind::Doc,
                body: TileBody::Empty,
                flags: TileFlagSet::empty(),
            },
        }
    }
}

impl Arena {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a tile, returning its id.
    pub fn insert(&mut self, tile: Tile) -> TileId {
        let id = TileId(u32::try_from(self.tiles.len()).unwrap_or(u32::MAX));
        self.tiles.push(Some(tile));
        id
    }

    /// Read access.
    ///
    /// # Panics
    ///
    /// An id that is out of range or names a vacant slot is a caller bug —
    /// a destroy that didn't rewire its parent. It yields the inert
    /// [`Arena::dead`] tile rather than panicking; see that field.
    #[must_use]
    pub fn get(&self, id: TileId) -> &Tile {
        self.tiles
            .get(id.as_usize())
            .and_then(std::option::Option::as_ref)
            .unwrap_or(&self.dead)
    }

    /// Mutable access. Same contract as [`Self::get`].
    pub fn get_mut(&mut self, id: TileId) -> &mut Tile {
        match self.tiles.get_mut(id.as_usize()) {
            Some(Some(tile)) => tile,
            _ => &mut self.dead,
        }
    }

    /// Iterate every live tile id in insertion order. Skips
    /// vacant slots.
    pub fn iter_ids(&self) -> impl Iterator<Item = TileId> + '_ {
        self.tiles.iter().enumerate().filter_map(|(i, slot)| {
            slot.as_ref()
                .map(|_| TileId(u32::try_from(i).unwrap_or(u32::MAX)))
        })
    }

    /// Total live tile count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tiles.iter().filter(|s| s.is_some()).count()
    }

    /// `true` when no live tiles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tiles.iter().all(std::option::Option::is_none)
    }
}

impl std::ops::Index<TileId> for Arena {
    type Output = Tile;
    fn index(&self, id: TileId) -> &Tile {
        self.get(id)
    }
}

impl std::ops::IndexMut<TileId> for Arena {
    fn index_mut(&mut self, id: TileId) -> &mut Tile {
        self.get_mut(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::flag::TileFlagSet;
    use crate::tile::{TileBody, TileKind};

    fn tile(kind: TileKind, length: usize) -> Tile {
        Tile {
            parent: None,
            children: Vec::new(),
            length,
            kind,
            body: TileBody::Empty,
            flags: TileFlagSet::empty(),
        }
    }

    #[test]
    fn insert_and_read_back() {
        let mut a = Arena::new();
        let id = a.insert(tile(TileKind::Line, 10));
        assert_eq!(a[id].length, 10);
        assert!(a[id].is_line());
    }

    #[test]
    fn iter_ids_yields_insert_order() {
        let mut a = Arena::new();
        let a_id = a.insert(tile(TileKind::Text, 1));
        let b_id = a.insert(tile(TileKind::Text, 2));
        let ids: Vec<_> = a.iter_ids().collect();
        assert_eq!(ids, vec![a_id, b_id]);
    }
}
