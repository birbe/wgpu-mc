use bytemuck::{Pod, Zeroable};
use cgmath::{Matrix4, SquareMatrix, Vector4};
use itertools::Itertools;
use minecraft_assets::api::ModelResolver;
use minecraft_assets::schemas;
use minecraft_assets::schemas::blockstates::ModelProperties;
use minecraft_assets::schemas::models::Textures;
use serde_derive::{Deserialize, Serialize};

use crate::mc::resource::ResourceProvider;
use crate::render::atlas::Atlas;
use crate::texture::UV;

use super::resource::ResourcePath;

/// A block position: x, y, z
pub type BlockPos = (i32, u16, i32);

#[derive(Hash, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct BlockstateKey {
    /// An index into [BlockManager]
    pub block: u16,
    /// Used to quickly figure out which [ModelMesh] a [Block] should return without having to hash strings
    pub augment: u16,
}

impl BlockstateKey {
    pub fn pack(&self) -> u32 {
        ((self.block as u32) << 16) | (self.augment as u32)
    }
}

impl From<(u16, u16)> for BlockstateKey {
    fn from(tuple: (u16, u16)) -> Self {
        Self {
            block: tuple.0,
            augment: tuple.1,
        }
    }
}

impl From<u32> for BlockstateKey {
    fn from(int: u32) -> Self {
        Self::from(((int >> 16) as u16, (int & 0xffff) as u16))
    }
}

///The state of one block, describing which variant
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChunkBlockState {
    Air,
    State(BlockstateKey),
}

impl ChunkBlockState {
    pub fn is_air(&self) -> bool {
        matches!(self, Self::Air)
    }
}

///Represents a vertex in a block mesh, including an additional UV offset index for animated textures.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct BlockMeshVertex {
    pub position: [f32; 3],
    pub tex_coords: [u16; 2],
    pub normal: [f32; 3],
    pub animation_uv_offset: u32,
}

#[derive(Debug, Copy, Clone)]
pub struct Face {
    pub vert_index: u32,
    pub tint_index: i32
}

#[derive(Debug)]
pub struct BlockModelFaces {
    pub vertices: [BlockMeshVertex; 24],
    pub north: Option<Face>,
    pub east: Option<Face>,
    pub south: Option<Face>,
    pub west: Option<Face>,
    pub up: Option<Face>,
    pub down: Option<Face>,
    pub cube: bool,
}

fn recurse_model_parents(
    model: &schemas::Model,
    resource_provider: &dyn ResourceProvider,
    models: &mut Vec<ResourcePath>,
) {
    match &model.parent {
        Some(parent_path_string) => {
            let parent_path: ResourcePath = ResourcePath::from(parent_path_string)
                .prepend("models/")
                .append(".json");
            recurse_model_parents(
                &serde_json::from_str(
                    &resource_provider
                        .get_string(&parent_path)
                        .expect(&parent_path.0),
                )
                .unwrap(),
                resource_provider,
                models,
            );
            models.push(parent_path);
        }
        None => {}
    }
}

fn resolve_model(
    model: schemas::Model,
    resource_provider: &dyn ResourceProvider,
) -> schemas::Model {
    if model.parent.is_none() {
        return model;
    }

    let mut parent_paths = Vec::new();
    recurse_model_parents(&model, resource_provider, &mut parent_paths);

    let parents: Vec<schemas::Model> = parent_paths
        .iter()
        .map(|parent_path| {
            serde_json::from_str(&resource_provider.get_string(parent_path).unwrap()).unwrap()
        })
        .collect();

    let mut schema = ModelResolver::resolve_model([&model].into_iter().chain(parents.iter()));

    if let Some(textures) = &mut schema.textures {
        let copy = textures.clone();

        textures.iter_mut().for_each(|(_key, texture)| {
            if texture.reference().is_some() {
                texture.0 = texture.resolve(&copy).unwrap().to_string();
            }
        })
    }

    schema
}

fn get_atlas_uv(face: &schemas::models::ElementFace, block_atlas: &Atlas, textures: &Option<Textures>) -> Option<UV> {
    let atlas_map = block_atlas.uv_map.read();

    // atlas_map.get(&(&face.texture.0).into()).copied().map(|uv| {
    //     let u = Vector2::new(uv.0.0 as i32, uv.0.1 as i32);
    //     let v = Vector2::new(uv.1.0 as i32, uv.1.1 as i32);
    //
    //     let d = v - u;
    //     let center = u + (d / 2);
    //
    //     let u_shift = u - center;
    //     let v_shift = v - center;
    //
    //     let matrix = match face.rotation {
    //         0 => Matrix2::new(1.0, 0.0, 0.0, 1.0),
    //         90 => Matrix2::new(0.0, 1.0, -1.0, 0.0),
    //         180 => Matrix2::new(-1.0, 0.0, 0.0, -1.0),
    //         270 => Matrix2::new(0.0, -1.0, 1.0, 0.0),
    //         _ => unreachable!()
    //     };
    //
    //     let u = matrix * u_shift.cast::<f32>().unwrap();
    //     let v = matrix * v_shift.cast::<f32>().unwrap();
    //
    //     ((u.x as u16 + center.x as u16, u.y as u16 + center.y as u16), (v.x as u16 + center.x as u16, v.y as u16 + center.y as u16))
    // })

    let texture_path: ResourcePath = match face.texture.reference() {
        None => {
            face.texture.0.clone().into()
        }
        Some(_) => {
            face.texture.resolve(
                textures.as_ref()?
            )?.into()
        }
    };

    atlas_map.get(&texture_path).copied()
}

pub struct RenderSettings {
    pub opaque: bool,
}

/// TODO: Use actual error handling library
#[derive(Debug)]
pub enum MeshBakeError {
    UnresolvedTextureReference(String),
    UnresolvedResourcePath(ResourcePath),
    JsonError(serde_json::Error),
}

/// A block model which has been baked into a mesh and is ready for rendering
/// The bool is true when the blocks next to this block should be rendered,
/// i.e. when this block does not fully obscure all six faces.
#[derive(Debug)]
pub struct ModelMesh {
    pub mesh: Vec<BlockModelFaces>,
    pub is_cube: bool,
}

impl ModelMesh {
    pub fn bake<'a>(
        model_properties: impl IntoIterator<Item = &'a ModelProperties>,
        resource_provider: &dyn ResourceProvider,
        block_atlas: &Atlas,
    ) -> Result<Self, MeshBakeError> {
        let mut all_elements_are_full_cubes = true;

        let mesh = model_properties.into_iter()
            .map(|model_properties: &ModelProperties| {
                let model_resource_path = ResourcePath::from(&model_properties.model).prepend("models/").append(".json");

                //Recursively resolve the model using it's parents if it has any
                let model: schemas::Model = resolve_model(
                    //Parse the JSON into the model schema
                    serde_json::from_str(
                        //Get the model JSON
                        &resource_provider.get_string(&model_resource_path)
                            .ok_or_else(|| MeshBakeError::UnresolvedResourcePath(model_resource_path.clone()))?
                    ).map_err(MeshBakeError::JsonError)?,
                    resource_provider
                );

                if let Some(ref textures) = model.textures {
                    //Make sure the textures in the model are fully resolved with no references
                    if let Some(reference) = textures.iter().find(|(_key, value)| value.reference().is_some()) { return Err(MeshBakeError::UnresolvedTextureReference(format!("key: {} value: {:?}", reference.0, reference.1))) }

                    let uv_map = block_atlas.uv_map.read();

                    let unallocated_textures: Vec<ResourcePath> = textures.iter()
                        .filter_map(|(_, texture)| {
                            let texture_id: ResourcePath = (&texture.0).into();
                            if !uv_map.contains_key(&texture_id) {
                                //Block UV atlas doesn't contain a texture, so we add it
                                Some(texture_id)
                            } else {
                                None
                            }
                        }).collect();

                    drop(uv_map);

                    let unallocated_textures: Vec<(&ResourcePath, Vec<u8>)> = unallocated_textures
                        .iter()
                        .map(|path| (path, resource_provider.get_bytes(&path.prepend("textures/").append(".png")).unwrap()))
                        .collect();

                    if !unallocated_textures.is_empty() {
                        block_atlas.allocate(
                            unallocated_textures.iter()
                                .map(|(path, data)| (*path, data)),
                            resource_provider,
                        );
                    }
                };

                // let matrix = Matrix4::from_angle_y(Deg(45.0));
                let matrix = Matrix4::identity();

                let results = model
                    .elements
                    .iter()
                    .flatten()
                    .map(|element| {
                        //Face textures

                        let tex_map = |&tex| {
                            get_atlas_uv(
                                tex,
                                block_atlas,
                                &model.textures
                            ).map(|uv| {
                                //TODO
                                let tex_uv = tex.uv.unwrap_or([0.0, 0.0, 16.0, 16.0]);

                                (
                                    //The default UV for this texture
                                    ((uv.0.0 + (tex_uv[0] as u16), uv.0.1 + (tex_uv[1] as u16)), ((uv.0.0 + (tex_uv[2] as u16), uv.0.1 + (tex_uv[3] as u16)))),
                                    //If this texture has an animation, get the offset, otherwise default to 0
                                    *block_atlas.animated_texture_offsets.read()
                                        .get(&(&tex.texture.0).into())
                                        .unwrap_or(&0),
                                    tex.tint_index
                                )
                            })
                        };

                        let up = element.faces.get(&schemas::models::BlockFace::Up);
                        let down = element.faces.get(&schemas::models::BlockFace::Down);
                        let north = element.faces.get(&schemas::models::BlockFace::North);
                        let east = element.faces.get(&schemas::models::BlockFace::East);
                        let west = element.faces.get(&schemas::models::BlockFace::West);
                        let south = element.faces.get(&schemas::models::BlockFace::South);

                        let north = north.as_ref().and_then(tex_map);
                        let east = east.as_ref().and_then(tex_map);
                        let south = south.as_ref().and_then(tex_map);
                        let west = west.as_ref().and_then(tex_map);
                        let up = up.as_ref().and_then(tex_map);
                        let down = down.as_ref().and_then(tex_map);

                        let a = (matrix * Vector4::new(1.0 - element.from[0] / 16.0, element.from[1] / 16.0, element.from[2] / 16.0, 1.0)).truncate().into();
                        let b = (matrix * Vector4::new(1.0 - element.to[0] / 16.0, element.from[1] / 16.0, element.from[2] / 16.0, 1.0)).truncate().into();
                        let c = (matrix * Vector4::new(1.0 - element.to[0] / 16.0, element.to[1] / 16.0, element.from[2] / 16.0, 1.0)).truncate().into();
                        let d = (matrix * Vector4::new(1.0 - element.from[0] / 16.0, element.to[1] / 16.0, element.from[2] / 16.0, 1.0)).truncate().into();
                        let e = (matrix * Vector4::new(1.0 - element.from[0] / 16.0, element.from[1] / 16.0, element.to[2] / 16.0, 1.0)).truncate().into();
                        let f = (matrix * Vector4::new(1.0 - element.to[0] / 16.0, element.from[1] / 16.0, element.to[2] / 16.0, 1.0)).truncate().into();
                        let g = (matrix * Vector4::new(1.0 - element.to[0] / 16.0, element.to[1] / 16.0, element.to[2] / 16.0, 1.0)).truncate().into();
                        let h = (matrix * Vector4::new(1.0 - element.from[0] / 16.0, element.to[1] / 16.0, element.to[2] / 16.0, 1.0)).truncate().into();

                        const NO_UV: (UV, u32, i32) = (((0, 0), (0, 0)), 0, -1);

                        //It's valid behavior for a face to not be defined in a block model. If that happens it won't be included
                        //in the chunk indices when rendering, but we need some placeholder, so we zero it out, which is fine because
                        //the face won't be taken into account when calculating chunk indices.
                        let north_face = north.unwrap_or(NO_UV);
                        let east_face = east.unwrap_or(NO_UV);
                        let south_face = south.unwrap_or(NO_UV);
                        let west_face = west.unwrap_or(NO_UV);
                        let up_face = up.unwrap_or(NO_UV);
                        let down_face = down.unwrap_or(NO_UV);

                        let current_element_is_full_cube = element.from[0] == 0.0
                            && element.from[1] == 0.0
                            && element.from[2] == 0.0
                            && element.to[0] == 16.0
                            && element.to[1] == 16.0
                            && element.to[2] == 16.0;

                        // if debug {
                        //     dbg!(north, down);
                        // }

                        #[rustfmt::skip]
                        let faces = BlockModelFaces {
                            vertices: [
                                BlockMeshVertex { position: h, tex_coords: [south_face.0.1.0, south_face.0.0.1], normal: [0.0, 0.0, 1.0], animation_uv_offset: south_face.1 },
                                BlockMeshVertex { position: g, tex_coords: [south_face.0.0.0, south_face.0.0.1], normal: [0.0, 0.0, 1.0], animation_uv_offset: south_face.1 },
                                BlockMeshVertex { position: f, tex_coords: [south_face.0.0.0, south_face.0.1.1], normal: [0.0, 0.0, 1.0], animation_uv_offset: south_face.1 },
                                BlockMeshVertex { position: e, tex_coords: [south_face.0.1.0, south_face.0.1.1], normal: [0.0, 0.0, 1.0], animation_uv_offset: south_face.1 },

                                BlockMeshVertex { position: f, tex_coords: [west_face.0.1.0, west_face.0.1.1], normal: [-1.0, 0.0, 0.0], animation_uv_offset: west_face.1 },
                                BlockMeshVertex { position: g, tex_coords: [west_face.0.1.0, west_face.0.0.1], normal: [-1.0, 0.0, 0.0], animation_uv_offset: west_face.1 },
                                BlockMeshVertex { position: c, tex_coords: [west_face.0.0.0, west_face.0.0.1], normal: [-1.0, 0.0, 0.0], animation_uv_offset: west_face.1 },
                                BlockMeshVertex { position: b, tex_coords: [west_face.0.0.0, west_face.0.1.1], normal: [-1.0, 0.0, 0.0], animation_uv_offset: west_face.1 },

                                BlockMeshVertex { position: a, tex_coords: [north_face.0.0.0, north_face.0.1.1], normal: [0.0, 0.0, -1.0], animation_uv_offset: north_face.1 },
                                BlockMeshVertex { position: b, tex_coords: [north_face.0.1.0, north_face.0.1.1], normal: [0.0, 0.0, -1.0], animation_uv_offset: north_face.1 },
                                BlockMeshVertex { position: c, tex_coords: [north_face.0.1.0, north_face.0.0.1], normal: [0.0, 0.0, -1.0], animation_uv_offset: north_face.1 },
                                BlockMeshVertex { position: d, tex_coords: [north_face.0.0.0, north_face.0.0.1], normal: [0.0, 0.0, -1.0], animation_uv_offset: north_face.1 },

                                BlockMeshVertex { position: h, tex_coords: [east_face.0.0.0, east_face.0.0.1], normal: [1.0, 0.0, 0.0], animation_uv_offset: east_face.1 },
                                BlockMeshVertex { position: e, tex_coords: [east_face.0.0.0, east_face.0.1.1], normal: [1.0, 0.0, 0.0], animation_uv_offset: east_face.1 },
                                BlockMeshVertex { position: a, tex_coords: [east_face.0.1.0, east_face.0.1.1], normal: [1.0, 0.0, 0.0], animation_uv_offset: east_face.1 },
                                BlockMeshVertex { position: d, tex_coords: [east_face.0.1.0, east_face.0.0.1], normal: [1.0, 0.0, 0.0], animation_uv_offset: east_face.1 },

                                BlockMeshVertex { position: d, tex_coords: [up_face.0.0.0, up_face.0.1.1], normal: [0.0, 1.0, 0.0], animation_uv_offset: up_face.1 },
                                BlockMeshVertex { position: c, tex_coords: [up_face.0.1.0, up_face.0.1.1], normal: [0.0, 1.0, 0.0], animation_uv_offset: up_face.1 },
                                BlockMeshVertex { position: g, tex_coords: [up_face.0.1.0, up_face.0.0.1], normal: [0.0, 1.0, 0.0], animation_uv_offset: up_face.1 },
                                BlockMeshVertex { position: h, tex_coords: [up_face.0.0.0, up_face.0.0.1], normal: [0.0, 1.0, 0.0], animation_uv_offset: up_face.1 },

                                BlockMeshVertex { position: e, tex_coords: [down_face.0.1.0, down_face.0.1.1], normal: [0.0, -1.0, 0.0], animation_uv_offset: down_face.1 },
                                BlockMeshVertex { position: f, tex_coords: [down_face.0.0.0, down_face.0.1.1], normal: [0.0, -1.0, 0.0], animation_uv_offset: down_face.1 },
                                BlockMeshVertex { position: b, tex_coords: [down_face.0.0.0, down_face.0.0.1], normal: [0.0, -1.0, 0.0], animation_uv_offset: down_face.1 },
                                BlockMeshVertex { position: a, tex_coords: [down_face.0.1.0, down_face.0.0.1], normal: [0.0, -1.0, 0.0], animation_uv_offset: down_face.1 },
                            ],
                            south: south.map(|(_, _, tint_index)| Face { vert_index: 0, tint_index}),
                            west: west.map(|(_, _, tint_index)| Face { vert_index: 4, tint_index}),
                            north: north.map(|(_, _, tint_index)| Face { vert_index: 8, tint_index}),
                            east: east.map(|(_, _, tint_index)| Face { vert_index: 12, tint_index}),
                            up: up.map(|(_, _, tint_index)| Face { vert_index: 16, tint_index}),
                            down: down.map(|(_, _, tint_index)| Face { vert_index: 20, tint_index}),
                            cube: current_element_is_full_cube,
                        };

                        all_elements_are_full_cubes &= current_element_is_full_cube;

                        Ok(faces)
                    }).collect::<Result<Vec<BlockModelFaces>, MeshBakeError>>();

                results
            })
            .flatten_ok()
            .collect::<Result<Vec<BlockModelFaces>, MeshBakeError>>()?;

        Ok(Self {
            mesh,
            is_cube: all_elements_are_full_cubes,
        })
    }
}
