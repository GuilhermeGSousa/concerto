//! The importer for BVH motion capture files.

/// [`BvhImporter`](bvh_importer::BvhImporter), which turns a `.bvh` source into a skeleton, an
/// animation clip and a scene, and its settings.
pub mod bvh_importer;
mod parser;
