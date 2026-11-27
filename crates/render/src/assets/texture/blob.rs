use anyhow::Context;
use serde::{Deserialize, Serialize};
use wgpu_types::{Extent3d, TextureFormat};

/// A texture's sampled levels, mip 0 first, each laid out in whole blocks of `format`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureLevels {
    /// The GPU format every level is in.
    pub format: TextureFormat,
    /// The width of level 0 in texels.
    pub width: u32,
    /// The height of level 0 in texels.
    pub height: u32,
    /// The bytes of each level, mip 0 first.
    pub levels: Vec<Vec<u8>>,
}

impl TextureLevels {
    /// The extent of level `level` in texels.
    pub fn extent(&self, level: u32) -> Extent3d {
        level_extent(self.width, self.height, level)
    }

    /// The bytes all levels take.
    pub fn byte_len(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    pub(crate) fn encode(&self) -> anyhow::Result<Vec<u8>> {
        bincode::serialize(self).context("failed to encode texture levels")
    }

    pub(crate) fn decode(blob: &[u8]) -> anyhow::Result<Self> {
        let levels: Self = bincode::deserialize(blob).context("failed to decode texture levels")?;
        levels.check()?;
        Ok(levels)
    }

    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.width > 0 && self.height > 0,
            "texture levels are {}x{}",
            self.width,
            self.height
        );
        let full_chain = 32 - self.width.max(self.height).leading_zeros();
        anyhow::ensure!(
            !self.levels.is_empty() && self.levels.len() as u32 <= full_chain,
            "a {}x{} texture can't have {} levels",
            self.width,
            self.height,
            self.levels.len()
        );
        for (level, bytes) in self.levels.iter().enumerate() {
            let extent = self.extent(level as u32);
            let expected = level_byte_len(self.format, extent)
                .with_context(|| format!("{:?} has no CPU layout", self.format))?;
            anyhow::ensure!(
                bytes.len() == expected,
                "texture level {level} holds {} bytes, but {}x{} {:?} takes {expected}",
                bytes.len(),
                extent.width,
                extent.height,
                self.format
            );
        }
        Ok(())
    }
}

pub(crate) fn level_extent(width: u32, height: u32, level: u32) -> Extent3d {
    Extent3d {
        width: (width >> level).max(1),
        height: (height >> level).max(1),
        depth_or_array_layers: 1,
    }
}

pub(crate) fn block_extent(format: TextureFormat, extent: Extent3d) -> Extent3d {
    let (block_width, block_height) = format.block_dimensions();
    Extent3d {
        width: extent.width.div_ceil(block_width) * block_width,
        height: extent.height.div_ceil(block_height) * block_height,
        depth_or_array_layers: 1,
    }
}

pub(crate) fn block_row_bytes(format: TextureFormat, extent: Extent3d) -> Option<u32> {
    let (block_width, _) = format.block_dimensions();
    Some(extent.width.div_ceil(block_width) * format.block_copy_size(None)?)
}

pub(crate) fn block_rows(format: TextureFormat, extent: Extent3d) -> u32 {
    let (_, block_height) = format.block_dimensions();
    extent.height.div_ceil(block_height)
}

pub(crate) fn level_byte_len(format: TextureFormat, extent: Extent3d) -> Option<usize> {
    Some(block_row_bytes(format, extent)? as usize * block_rows(format, extent) as usize)
}
