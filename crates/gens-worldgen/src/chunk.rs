//! Chunk addressing and chunk content generation.
//!
//! A chunk is a square world region addressed by integer coordinates;
//! its content is a **pure function of `(seed, coord)`** (ADR-0031):
//! terrain sampled from the height noise at WORLD coordinates (so
//! neighboring chunks join seamlessly), plus a scatter plan over the
//! same region. The generator holds no mutable state — generating the
//! same coord twice gives identical data, and generation order cannot
//! matter because nothing is shared between calls.

use gens_math::Vec3;
use thiserror::Error;

use crate::heightfield::HeightField;
use crate::noise::Noise2D;
use crate::scatter::ScatterPoint;

/// Integer chunk coordinates, ordered row-major (z, then x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkCoord {
    /// X index (east-west).
    pub x: i32,
    /// Z index (north-south).
    pub z: i32,
}

impl ChunkCoord {
    /// Builds a coordinate pair.
    #[must_use]
    pub fn new(x: i32, z: i32) -> ChunkCoord {
        ChunkCoord { x, z }
    }
}

/// Errors from generator construction.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum WorldGenError {
    /// Chunks need at least 2x2 samples to carry triangles.
    #[error("chunk samples must be >= 2, got {samples}")]
    TooFewSamples {
        /// The offending sample count.
        samples: u32,
    },
    /// Cell size must be finite and > 0.
    #[error("cell size must be finite and > 0, got {cell_size}")]
    InvalidCellSize {
        /// The offending cell size.
        cell_size: f32,
    },
    /// FBM parameters were rejected (see [`crate::noise::NoiseError`]).
    #[error("invalid noise parameters: {0}")]
    InvalidNoise(#[from] crate::noise::NoiseError),
    /// A terrain sample rejected the generated content (see
    /// [`crate::heightfield::HeightFieldError`]).
    #[error("terrain sampling failed: {0}")]
    Terrain(#[from] crate::heightfield::HeightFieldError),
    /// A scatter plan was rejected (see [`crate::scatter::ScatterError`]).
    #[error("scatter failed: {0}")]
    Scatter(#[from] crate::scatter::ScatterError),
}

/// The caller-supplied generator parameters, validated by
/// [`WorldGen::from_config`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldGenConfig {
    /// Master seed; all sub-noises derive from it.
    pub seed: u64,
    /// Terrain samples per chunk edge (the mesh has `samples + 1` per
    /// side so neighbors share their border row).
    pub samples_per_chunk: u32,
    /// World distance between terrain samples.
    pub cell_size: f32,
    /// World-to-noise coordinate scale (higher = rougher terrain).
    pub frequency: f32,
    /// FBM octave count.
    pub octaves: u32,
    /// FBM frequency multiplier per octave.
    pub lacunarity: f32,
    /// FBM amplitude scale per octave.
    pub gain: f32,
    /// Density threshold: cells whose density noise clears it keep a prop.
    pub density_threshold: f32,
}

/// Validated generator — construct once, generate anywhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldGen {
    config: WorldGenConfig,
    height_noise: Noise2D,
    density_noise: Noise2D,
    scatter_seed: u64,
}

impl WorldGen {
    /// Builds and validates a generator from its configuration.
    ///
    /// # Errors
    /// [`WorldGenError`] on too-few samples, invalid cell size, or FBM
    /// parameter rejection.
    pub fn from_config(config: WorldGenConfig) -> Result<WorldGen, WorldGenError> {
        if config.samples_per_chunk < 2 {
            return Err(WorldGenError::TooFewSamples {
                samples: config.samples_per_chunk,
            });
        }
        if !config.cell_size.is_finite() || config.cell_size <= 0.0 {
            return Err(WorldGenError::InvalidCellSize {
                cell_size: config.cell_size,
            });
        }
        // Validate the FBM ladder once, at construction.
        Noise2D::new(config.seed).fbm(0.0, 0.0, config.octaves, config.lacunarity, config.gain)?;
        Ok(WorldGen {
            height_noise: Noise2D::new(config.seed),
            density_noise: Noise2D::new(config.seed ^ 0x5EED_5EED_5EED_5EED),
            scatter_seed: config.seed ^ 0x5CA7_7E45_0000_0001,
            config,
        })
    }

    /// The validated configuration.
    #[must_use]
    pub fn config(&self) -> &WorldGenConfig {
        &self.config
    }

    /// The world-space size of one chunk edge.
    #[must_use]
    pub fn chunk_size(&self) -> f32 {
        self.config.samples_per_chunk as f32 * self.config.cell_size
    }

    /// World-space origin corner of `coord`.
    #[must_use]
    pub fn origin(&self, coord: ChunkCoord) -> [f32; 2] {
        [
            coord.x as f32 * self.chunk_size(),
            coord.z as f32 * self.chunk_size(),
        ]
    }

    /// The chunk a world position falls into.
    #[must_use]
    pub fn coord_of(&self, wx: f32, wz: f32) -> ChunkCoord {
        let s = self.chunk_size();
        ChunkCoord::new((wx / s).floor() as i32, (wz / s).floor() as i32)
    }

    /// The world-space terrain height at `(wx, wz)` — the same function
    /// the terrain sampler uses, exposed for prop lifting and callers
    /// that need to place things on the ground.
    ///
    /// # Errors
    /// [`WorldGenError::InvalidNoise`] (already validated at
    /// construction, so in practice unreachable — kept for honesty).
    pub fn height_at(&self, wx: f32, wz: f32) -> Result<f32, WorldGenError> {
        Ok(self.height_noise.fbm(
            wx * self.config.frequency,
            wz * self.config.frequency,
            self.config.octaves,
            self.config.lacunarity,
            self.config.gain,
        )? * 12.0)
    }

    /// Generates the full content of one chunk.
    ///
    /// Terrain samples include the +edge row (`samples + 1` per side)
    /// so neighboring chunks agree on their shared border heights. The
    /// mesh is chunk-local (the origin carries the placement) so the
    /// content hash addresses the chunk's shape; scatter points are
    /// world-space.
    ///
    /// # Errors
    /// [`WorldGenError`] if any sampled height is non-finite (cannot
    /// happen with this generator's own noise, but the closure path is
    /// shared with external samplers).
    pub fn generate(&self, coord: ChunkCoord) -> Result<ChunkContent, WorldGenError> {
        let [ox, oz] = self.origin(coord);
        let n = self.config.samples_per_chunk;
        let height_field = HeightField::sample(n + 1, n + 1, self.config.cell_size, |dx, dz| {
            // dx/dz are chunk-LOCAL offsets; sample the noise at WORLD
            // coordinates so chunks join seamlessly.
            let wx = ox + dx;
            let wz = oz + dz;
            match self.height_noise.fbm(
                wx * self.config.frequency,
                wz * self.config.frequency,
                self.config.octaves,
                self.config.lacunarity,
                self.config.gain,
            ) {
                Ok(v) => v * 12.0,
                // Cannot happen: the FBM ladder is validated at
                // construction. NaN routes through HeightField::sample's
                // loud NonFiniteHeight error instead of a silent default.
                Err(_) => f32::NAN,
            }
        })?;
        // The mesh stays CHUNK-LOCAL: vertices are local positions, the
        // origin field positions it in the world. (Local vertices keep
        // float precision at far world coordinates and make the content
        // hash address the chunk's shape, not its placement.)
        let mesh = height_field.to_mesh();
        let scatter = crate::scatter::scatter(
            self.scatter_seed,
            &self.density_noise,
            self.config.density_threshold,
            self.chunk_size(),
            self.chunk_size(),
            self.config.cell_size * 2.0,
            |lx, lz| {
                self.height_at(ox + lx, oz + lz)
                    .expect("fbm params validated at construction")
            },
        )?;
        // Scatter points are WORLD-space (shift the lattice output);
        // their y already came from the world-space height closure.
        let scatter = scatter
            .points
            .into_iter()
            .map(|mut p| {
                p.position[0] += ox;
                p.position[2] += oz;
                p
            })
            .collect();
        Ok(ChunkContent {
            coord,
            origin: Vec3::new(ox, 0.0, oz),
            mesh,
            scatter,
        })
    }
}

/// The content one chunk generates.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkContent {
    /// The chunk this content belongs to.
    pub coord: ChunkCoord,
    /// World-space origin corner — positions the chunk-local mesh.
    pub origin: Vec3,
    /// Terrain mesh (CHUNK-LOCAL vertices, content-hashed by shape).
    pub mesh: crate::heightfield::TerrainMesh,
    /// Scattered props (world-space positions).
    pub scatter: Vec<ScatterPoint>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> WorldGenConfig {
        WorldGenConfig {
            seed: 42,
            samples_per_chunk: 8,
            cell_size: 1.0,
            frequency: 0.08,
            octaves: 4,
            lacunarity: 2.0,
            gain: 0.5,
            density_threshold: 0.4,
        }
    }

    fn generator() -> WorldGen {
        WorldGen::from_config(config()).expect("valid config")
    }

    #[test]
    fn validates_configuration() {
        let bad_samples = WorldGenConfig {
            samples_per_chunk: 1,
            ..config()
        };
        assert_eq!(
            WorldGen::from_config(bad_samples).unwrap_err(),
            WorldGenError::TooFewSamples { samples: 1 }
        );
        let bad_cell = WorldGenConfig {
            cell_size: 0.0,
            ..config()
        };
        assert_eq!(
            WorldGen::from_config(bad_cell).unwrap_err(),
            WorldGenError::InvalidCellSize { cell_size: 0.0 }
        );
        let bad_fbm = WorldGenConfig {
            octaves: 0,
            ..config()
        };
        assert!(matches!(
            WorldGen::from_config(bad_fbm),
            Err(WorldGenError::InvalidNoise(_))
        ));
    }

    #[test]
    fn generation_is_pure_in_coord() {
        let g = generator();
        let a = g.generate(ChunkCoord::new(0, 0)).expect("ok");
        let b = g.generate(ChunkCoord::new(0, 0)).expect("ok");
        assert_eq!(a, b, "same coord, same content, any call order");
        let c = g.generate(ChunkCoord::new(-1, 2)).expect("ok");
        let d = g.generate(ChunkCoord::new(-1, 2)).expect("ok");
        assert_eq!(c, d);
        assert_ne!(
            a.mesh.hash, c.mesh.hash,
            "different coords, different terrain"
        );
    }

    #[test]
    fn neighboring_chunks_agree_on_the_shared_edge() {
        let g = generator();
        let left = g.generate(ChunkCoord::new(0, 0)).expect("ok");
        let right = g.generate(ChunkCoord::new(1, 0)).expect("ok");
        // Chunks are local: the left chunk's +X edge column (local x =
        // 8 * cell) and the right chunk's -X edge column (local x = 0)
        // sample the same WORLD heights, one cell_size apart in local x.
        let samples = 9; // samples_per_chunk 8 + 1
        let cell = 1.0f32;
        for z in 0..samples {
            // Row-major index: position = z * samples + x.
            let l = &left.mesh.positions[z * samples + (samples - 1)];
            let r = &right.mesh.positions[z * samples];
            assert_eq!(l[0], 8.0 * cell, "left +X edge is at local x=8");
            assert_eq!(r[0], 0.0, "right -X edge is at local x=0");
            assert_eq!(l[1], r[1], "shared-edge WORLD height matches");
            assert_eq!(l[2], r[2], "shared-edge z matches");
        }
    }

    #[test]
    fn coord_of_follows_floor_semantics() {
        let g = generator(); // chunk_size = 8
        assert_eq!(g.coord_of(0.0, 0.0), ChunkCoord::new(0, 0));
        assert_eq!(g.coord_of(7.9, 7.9), ChunkCoord::new(0, 0));
        assert_eq!(g.coord_of(8.0, 0.0), ChunkCoord::new(1, 0));
        assert_eq!(g.coord_of(-0.1, -0.1), ChunkCoord::new(-1, -1));
    }

    #[test]
    fn scatter_lands_on_the_terrain() {
        let g = generator();
        let content = g.generate(ChunkCoord::new(0, 0)).expect("ok");
        for p in &content.scatter {
            let expected = g.height_at(p.position[0], p.position[2]).expect("ok");
            assert_eq!(p.position[1], expected, "prop y sits on the terrain");
            // Inside the chunk's world region.
            let [ox, oz] = g.origin(content.coord);
            assert!((ox..ox + g.chunk_size()).contains(&p.position[0]));
            assert!((oz..oz + g.chunk_size()).contains(&p.position[2]));
        }
    }
}
