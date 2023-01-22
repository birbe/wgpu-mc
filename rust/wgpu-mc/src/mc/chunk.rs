//! # Everything regarding minecraft chunks
//!
//! This handles storing the state of all blocks in a chunk, as well as making
//! baking the chunk mesh
//!
//! # Chunk sections?
//!
//! Minecraft splits chunks into 16-block tall pieces called chunk sections, for
//! rendering purposes.

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};
use range_alloc::RangeAllocator;
use std::collections::HashMap;
use std::fmt::Debug;
use std::mem::size_of;
use std::ops::Range;
use std::sync::Arc;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{BufferAddress, BufferDescriptor, BufferUsages};

use crate::mc::block::{
    BlockMeshVertex, BlockstateKey, ChunkBlockState, CubeOrComplexMesh, ModelMesh,
};
use crate::mc::BlockManager;
use crate::render::pipeline::Vertex;

use crate::{WgpuState, WmRenderer};

pub const CHUNK_WIDTH: usize = 16;
pub const CHUNK_AREA: usize = CHUNK_WIDTH * CHUNK_WIDTH;
pub const CHUNK_HEIGHT: usize = 384;
pub const CHUNK_VOLUME: usize = CHUNK_AREA * CHUNK_HEIGHT;
pub const CHUNK_SECTION_HEIGHT: usize = 16;
pub const CHUNK_SECTIONS_PER: usize = CHUNK_HEIGHT / CHUNK_SECTION_HEIGHT;
pub const SECTION_VOLUME: usize = CHUNK_AREA * CHUNK_SECTION_HEIGHT;

pub const CHUNK_ALLOCATOR_SIZE: usize = 1000 * 1000 * 1000;

pub type ChunkPos = [i32; 2];

pub struct ChunkAllocation {
    pub buffer: Arc<wgpu::Buffer>,
    pub allocator: RwLock<RangeAllocator<usize>>,
}

pub struct ChunkManager {
    pub loaded_chunks: RwLock<HashMap<ChunkPos, ArcSwap<Chunk>>>,
    pub chunk_offset: Mutex<ChunkPos>,
    pub chunk_allocation: ChunkAllocation,
}

impl ChunkManager {
    #[must_use]
    pub fn new(wgpu_state: &WgpuState) -> Self {
        ChunkManager {
            loaded_chunks: RwLock::new(HashMap::new()),
            chunk_offset: Mutex::new([0, 0]),
            chunk_allocation: ChunkAllocation {
                buffer: Arc::new(wgpu_state.device.create_buffer(&BufferDescriptor {
                    label: None,
                    size: CHUNK_ALLOCATOR_SIZE as BufferAddress,
                    usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })),
                allocator: RwLock::new(RangeAllocator::new(0..CHUNK_ALLOCATOR_SIZE)),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChunkSection {
    pub empty: bool,
    pub blocks: Box<[ChunkBlockState; SECTION_VOLUME]>,
    pub offset_y: usize,
}

/// Return a BlockState within the provided world coordinates.
pub trait BlockStateProvider: Send + Sync + Debug {
    fn get_state(&self, x: i32, y: i16, z: i32) -> ChunkBlockState;

    fn is_section_empty(&self, index: usize) -> bool;
}

pub trait RenderLayer: Send + Sync {
    fn filter(&self) -> fn(BlockstateKey) -> bool;

    fn mapper(&self) -> fn(&BlockMeshVertex, f32, f32, f32) -> Vertex;

    fn name(&self) -> &str;
}

/// A representation of a chunk, containing buffers and vertices for rendering.
#[derive(Debug)]
pub struct Chunk {
    pub pos: ChunkPos,
    /// The layers here don't have to be sections, and the [String] keys are used to distinguish
    /// which [RenderLayer] the vertices come from.
    pub baked_layers: RwLock<HashMap<String, Range<usize>>>,
}

impl Chunk {
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            baked_layers: Default::default(),
        }
    }

    /// Bakes the layers, and uploads them to the GPU.
    pub fn bake_chunk<T: BlockStateProvider>(
        &self,
        wm: &WmRenderer,
        layers: &[Box<dyn RenderLayer>],
        block_manager: &BlockManager,
        provider: &T,
    ) {
        let baked_layers = layers
            .iter()
            .map(|layer| {
                let verts = bake_layer(
                    block_manager,
                    self,
                    layer.mapper(),
                    layer.filter(),
                    provider,
                );

                let range = {
                    let mut allocator = wm.mc.chunks.chunk_allocation.allocator.write();
                    let size = verts.len() * size_of::<Vertex>();
                    allocator.allocate_range(size).unwrap()
                };

                wm.wgpu_state.queue.write_buffer(
                    &wm.mc.chunks.chunk_allocation.buffer,
                    range.start as u64,
                    bytemuck::cast_slice(&verts)
                );

                (layer.name().into(), range)
            })
            .collect();

        *self.baked_layers.write() = baked_layers;
    }
}

/// Returns true if the block at the given coordinates is either not a full cube or has transparency
#[inline]
fn is_block_not_fully_opaque(
    block_manager: &BlockManager,
    state_provider: &impl BlockStateProvider,
    x: i32,
    y: i16,
    z: i32,
) -> bool {
    let state = get_block(block_manager, state_provider.get_state(x, y, z));

    match state {
        Some(mesh) => mesh.models[0].1,
        None => true,
    }
}

fn get_block(block_manager: &BlockManager, state: ChunkBlockState) -> Option<Arc<ModelMesh>> {
    let key = match state {
        ChunkBlockState::Air => return None,
        ChunkBlockState::State(key) => key,
    };

    Some(
        block_manager
            .blocks
            .get_index(key.block as usize)?
            .1
            .get_model(key.augment),
    )
}

pub fn bake_layer<
    T,
    Provider: BlockStateProvider,
    Filter: Fn(BlockstateKey) -> bool,
    Mapper: Fn(&BlockMeshVertex, f32, f32, f32) -> T,
>(
    block_manager: &BlockManager,
    chunk: &Chunk,
    mapper: Mapper,
    filter: Filter,
    state_provider: &Provider,
) -> (Vec<T>, Vec<u32>) {
    //Generates the mesh for this chunk, culling faces whenever possible
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    let mut block_index = 0;

    loop {
        if block_index >= CHUNK_VOLUME {
            break;
        }

        let x = (block_index % CHUNK_WIDTH) as i32;
        let y = (block_index / CHUNK_AREA) as i16;
        let z = ((block_index % CHUNK_AREA) / CHUNK_WIDTH) as i32;

        if x == 0 && z == 0 && (y as usize % CHUNK_SECTION_HEIGHT) == 0 {
            let section_index = y as usize / CHUNK_SECTION_HEIGHT;
            if state_provider.is_section_empty(section_index) {
                block_index += CHUNK_SECTION_HEIGHT * CHUNK_AREA;
                continue;
            }
        }

        block_index += 1;

        let absolute_x = (chunk.pos[0] * 16) + x;
        let absolute_z = (chunk.pos[1] * 16) + z;

        let block_state: ChunkBlockState = state_provider.get_state(absolute_x, y, absolute_z);

        if block_state.is_air() {
            continue;
        }

        let state_key = match block_state {
            ChunkBlockState::Air => unreachable!(),
            ChunkBlockState::State(key) => key,
        };

        if !filter(state_key) {
            continue;
        }

        let mesh = get_block(block_manager, block_state).unwrap();

        // TODO: randomly select a mesh if there are multiple

        match &mesh.models[0].0 {
            CubeOrComplexMesh::Cube(model) => {
                let baked_should_render_face = |x_: i32, y_: i16, z_: i32| {
                    is_block_not_fully_opaque(block_manager, state_provider, x_, y_, z_)
                };

                let render_east = baked_should_render_face(absolute_x + 1, y, absolute_z);
                let render_west = baked_should_render_face(absolute_x - 1, y, absolute_z);
                let render_up = baked_should_render_face(absolute_x, y + 1, absolute_z);
                let render_down = baked_should_render_face(absolute_x, y - 1, absolute_z);
                let render_south = baked_should_render_face(absolute_x, y, absolute_z + 1);
                let render_north = baked_should_render_face(absolute_x, y, absolute_z - 1);

                let add_face = || {
                    render_east
                };

                if render_north {

                }
                if render_east {

                }
                if render_south {

                }
                if render_west {

                }
                if render_up {

                }
                if render_down {

                }
            }
            CubeOrComplexMesh::Complex(model) => {
                vertices.extend(
                    model
                        .iter()
                        .flat_map(|faces| {
                            [
                                faces.north.as_ref(),
                                faces.east.as_ref(),
                                faces.south.as_ref(),
                                faces.west.as_ref(),
                                faces.up.as_ref(),
                                faces.down.as_ref(),
                            ]
                        })
                        .flatten()
                        .flatten()
                        .map(|v| mapper(v, x as f32, y as f32, z as f32)),
                );
            }
        }
    }

    vertices
}
