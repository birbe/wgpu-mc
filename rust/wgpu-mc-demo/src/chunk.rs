use std::fmt::Debug;
use std::sync::Arc;
use std::time::Instant;

use wgpu_mc::mc::block::{BlockstateKey, ChunkBlockState};
use wgpu_mc::mc::chunk::{BlockStateProvider, Chunk, LightLevel};
use wgpu_mc::mc::MinecraftState;
use wgpu_mc::minecraft_assets::schemas::blockstates::multipart::StateValue;
use wgpu_mc::render::pipeline::BLOCK_ATLAS;
use wgpu_mc::WmRenderer;

struct SimpleBlockstateProvider(Arc<MinecraftState>, BlockstateKey, u16);

impl BlockStateProvider for SimpleBlockstateProvider {
    fn get_state(&self, x: i32, y: i16, z: i32) -> ChunkBlockState {
        if (0..1).contains(&x) && (0..1).contains(&z) && y == 0 {
            ChunkBlockState::State(self.1)
        } else {
            ChunkBlockState::Air
        }
    }

    fn get_light_level(&self, _x: i32, _y: i16, _z: i32) -> LightLevel {
        LightLevel::from_sky_and_block(15, 15)
    }

    fn get_block_color(&self, x: i32, y: i16, z: i32, tint_index: i32) -> [u8; 3] {
        let block = if let ChunkBlockState::State(state) = self.get_state(x, y, z) {
            state.block
        } else {
            return [255; 3];
        };

        if block == self.2 {
            [39, 114, 40]
        } else {
            [255; 3]
        }
    }

    fn is_section_empty(&self, _index: usize) -> bool {
        false
    }
}

impl Debug for SimpleBlockstateProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("")
    }
}

pub fn make_chunks(wm: &WmRenderer) -> Chunk {
    let bm = wm.mc.block_manager.read();
    let atlas = wm
        .mc
        .texture_manager
        .atlases
        .load()
        .get(BLOCK_ATLAS)
        .unwrap()
        .load();

    let (index, _, block) = bm.blocks.get_full("minecraft:anvil").unwrap();

    // let (grass_index, _, _) = bm.blocks.get_full("minecraft:grass_block").unwrap();

    // dbg!(&block);

    let (mesh, augment) = block
        .get_model_by_key(
            [
                ("facing", &StateValue::String("north".into())),
                // ("lit", &StateValue::Bool(true)),
                // ("snowy", &StateValue::Bool(false)),
                // ("layers", &StateValue::String("1".into())),
            ],
            &*wm.mc.resource_provider,
            &atlas,
            0,
        )
        .unwrap();

    let provider = SimpleBlockstateProvider(
        wm.mc.clone(),
        BlockstateKey {
            block: index as u16,
            augment,
        },
        999
    );

    let chunk = Chunk::new([0, 0]);
    let time = Instant::now();

    let pipelines = wm.pipelines.load();
    let layers = pipelines.chunk_layers.load();

    chunk.bake_chunk(wm, &layers, &bm, &provider);

    println!(
        "Built 1 chunk in {} microseconds",
        Instant::now().duration_since(time).as_micros()
    );

    chunk
}
