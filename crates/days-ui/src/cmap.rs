//! `.CMAP` — the per-pixel hit map that names the widget under the cursor.
//!
//! ```text
//! width   u32 LE
//! height  u32 LE
//! pixels  [u8; width * height]   region ID, 0 = no region
//! ```
//!
//! Hit-testing a FILMEngine screen is a pixel lookup, not a geometry test, so a
//! widget can be any shape at all — the route map screens use that heavily.
//! Region IDs are dense from 1.

use crate::Error;

/// A rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A parsed hit map.
#[derive(Debug)]
pub struct Cmap {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    /// Bounding box per region ID, indexed by `id - 1`.
    bounds: Vec<Rect>,
}

impl Cmap {
    /// Parses a `.CMAP`.
    pub fn parse(bytes: &[u8]) -> Result<Cmap, Error> {
        let header: [u8; 8] = bytes
            .get(..8)
            .ok_or(Error::TruncatedCmap)?
            .try_into()
            .unwrap();
        let width = u32::from_le_bytes(header[..4].try_into().unwrap());
        let height = u32::from_le_bytes(header[4..].try_into().unwrap());
        let want = (width as usize)
            .checked_mul(height as usize)
            .ok_or(Error::TruncatedCmap)?;
        let pixels = bytes.get(8..8 + want).ok_or(Error::TruncatedCmap)?.to_vec();

        // Accumulate each region's extent in one pass; screens have at most a
        // few dozen regions but the map itself can be a megabyte.
        let mut min = Vec::<[u32; 4]>::new();
        for y in 0..height {
            for x in 0..width {
                let id = pixels[(y * width + x) as usize];
                if id == 0 {
                    continue;
                }
                let i = id as usize - 1;
                if min.len() <= i {
                    min.resize(i + 1, [u32::MAX, u32::MAX, 0, 0]);
                }
                let b = &mut min[i];
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
        let bounds = min
            .iter()
            .map(|b| {
                if b[0] == u32::MAX {
                    Rect {
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                    }
                } else {
                    Rect {
                        x: b[0],
                        y: b[1],
                        width: b[2] - b[0] + 1,
                        height: b[3] - b[1] + 1,
                    }
                }
            })
            .collect();

        Ok(Cmap {
            width,
            height,
            pixels,
            bounds,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Number of distinct region IDs, i.e. the highest ID present.
    pub fn region_count(&self) -> usize {
        self.bounds.len()
    }

    /// The region ID under a point, or 0 for none. Out of range reads as 0.
    pub fn region_at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.pixels[(y * self.width + x) as usize]
    }

    /// The bounding box of a region, or `None` if that ID is not present.
    pub fn bounds(&self, id: u8) -> Option<Rect> {
        if id == 0 {
            return None;
        }
        self.bounds
            .get(id as usize - 1)
            .copied()
            .filter(|r| r.width > 0)
    }

    /// Every region's bounding box, in ID order starting at 1.
    pub fn all_bounds(&self) -> &[Rect] {
        &self.bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(width: u32, height: u32, pixels: Vec<u8>) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&width.to_le_bytes());
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(&pixels);
        b
    }

    #[test]
    fn bounds_come_from_the_extent_of_each_id() {
        // 4x3, region 1 across the top-left 2x2, region 2 one pixel at (3, 2).
        let c = Cmap::parse(&map(4, 3, vec![1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 2])).unwrap();
        assert_eq!(c.region_count(), 2);
        assert_eq!(
            c.bounds(1),
            Some(Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2
            })
        );
        assert_eq!(
            c.bounds(2),
            Some(Rect {
                x: 3,
                y: 2,
                width: 1,
                height: 1
            })
        );
        assert_eq!(c.bounds(3), None);
    }

    #[test]
    fn a_lookup_outside_the_map_is_no_region_rather_than_a_panic() {
        let c = Cmap::parse(&map(2, 2, vec![1, 1, 1, 1])).unwrap();
        assert_eq!(c.region_at(0, 0), 1);
        assert_eq!(c.region_at(9, 9), 0);
    }

    #[test]
    fn a_short_file_is_an_error_not_a_partial_map() {
        assert!(Cmap::parse(&map(4, 4, vec![0; 3])).is_err());
        assert!(Cmap::parse(&[0u8; 4]).is_err());
    }
}
