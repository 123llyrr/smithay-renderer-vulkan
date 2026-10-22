pub mod blur;
pub mod damage_region;
pub mod particles;
pub mod shaders;
pub mod sync;

pub mod vulkan {
    pub use crate::*;
}

/// Helper function to copy pixel bytes from a mapped texture into a Vec.
pub fn mapping_to_vec<R>(renderer: &mut R, mapping: &R::TextureMapping) -> anyhow::Result<Vec<u8>>
where
    R: smithay::backend::renderer::ExportMem,
    R::Error: Send + Sync + 'static,
{
    use anyhow::Context as _;
    let copy = renderer
        .map_texture(mapping)
        .context("error mapping texture")?;
    Ok(copy.to_vec())
}

