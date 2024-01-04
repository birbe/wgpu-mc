struct CameraUniform {
    view: mat4x4<f32>
};

struct UV {
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    blend: f32,
    padding: f32
};

struct UVs {
    uvs: array<UV>
};

struct ChunkOffset {
    x: i32,
    z: i32
}

struct PushConstants {
    chunk_x: i32,
    chunk_y: i32,
    chunk_z: i32,
    fb_width: f32,
    fb_height: f32
}

var<push_constant> push_constants: PushConstants;

@group(0) @binding(0) var<uniform> camera_uniform: CameraUniform;

@group(1) @binding(0) var t_texture: texture_2d<f32>;
@group(1) @binding(1) var t_sampler: sampler;

@group(2) @binding(0) var<storage> vertex_data: array<u32>;
@group(3) @binding(0) var<storage> index_data: array<u32>;

@group(4) @binding(0) var lightmap_texture: texture_2d<f32>;
@group(4) @binding(1) var lightmap_sampler: sampler;

@group(5) @binding(0) var<uniform> proj: mat4x4<f32>;

struct VertexResult {
    @builtin(position) pos: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) tex_coords2: vec2<f32>,
    @location(2) blend: f32,
    @location(3) normal: vec3<f32>,
    @location(4) world_pos: vec3<f32>,
    @location(5) @interpolate(flat) light_coords: vec2<f32>,
    @location(6) color: vec3<f32>
};

@vertex
fn vert(
    @builtin(vertex_index) vertex_index: u32
) -> VertexResult {
    // var uv = uv_offsets.uvs[uv_offset];

    var vr: VertexResult;

    var index: u32 = index_data[vertex_index];

    var v1 = vertex_data[index * 4u];
    var v2 = vertex_data[(index * 4u) + 1u];
    var v3 = vertex_data[(index * 4u) + 2u];
    var v4 = vertex_data[(index * 4u) + 3u];

    var x: f32 = f32(v1 & 0xffu) * 0.0625;
    var y: f32 = f32((v1 >> 8u) & 0xffu) * 0.0625;
    var z: f32 = f32((v1 >> 16u) & 0xffu) * 0.0625;

    var r: f32 = f32(v1 >> 24u) / 255.0;
    var g: f32 = f32(v2 & 0xffu) / 255.0;
    var b: f32 = f32((v2 >> 8u) & 0xffu) / 255.0;

    var u: f32 = f32((v2 >> 16u) & 0xffffu) * 0.00048828125;
    var v: f32 = f32(v3 & 0xffffu) * 0.00048828125;


    if(((v3 >> 61u) & 1u) == 1u) {
        x = 16.0;
    }

    if(((v3 >> 62u) & 1u) == 1u) {
        y = 16.0;
    }

    if((v3 >> 63u) == 1u) {
        z = 16.0;
    }

    var pos = vec3<f32>(x, y, z);

    var world_pos = pos + vec3<f32>(f32(push_constants.chunk_x) * 16.0, f32(push_constants.chunk_y) * 16.0, f32(push_constants.chunk_z) * 16.0);

    vr.pos = proj * camera_uniform.view * vec4(world_pos, 1.0);
    vr.tex_coords = vec2<f32>(u, v);
    vr.tex_coords2 = vec2(0.0, 0.0);
    vr.world_pos = world_pos;
    vr.color = vec3(r, g, b);

    var light_coords = vec2<u32>(v4 & 15u, (v4 >> 4u) & 15u);
    vr.light_coords = vec2(f32(light_coords.x) / 15.0, f32(light_coords.y) / 15.0);

    vr.blend = 0.0;

    return vr;
}

fn minecraft_sample_lighting(uv: vec2<u32> ) -> f32 {
    return f32(max(uv.x, uv.y)) / 15.0;
}

@fragment
fn frag(
    in: VertexResult
) -> @location(0) vec4<f32> {
    let col1 = textureSample(t_texture, t_sampler, in.tex_coords);

//    let light = textureSample(lightmap_texture, lightmap_sampler, vec2(max(in.light_coords.x, in.light_coords.y), 0.0));
    let light = max(in.light_coords.x, in.light_coords.y);
    let color = vec4(in.color, 1.0);

    return vec4(light, light, light, 1.0) * color * col1;
}
