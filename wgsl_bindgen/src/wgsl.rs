use naga::StructMember;
use proc_macro2::TokenStream;
use quote::quote;

use crate::quote_gen::RustItemPath;

pub fn shader_stages(module: &naga::Module) -> wgpu::ShaderStages {
    module
        .entry_points
        .iter()
        .map(|entry| match entry.stage {
            naga::ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
            naga::ShaderStage::Fragment => wgpu::ShaderStages::FRAGMENT,
            naga::ShaderStage::Compute => wgpu::ShaderStages::COMPUTE,
        })
        .collect()
}

pub fn buffer_binding_type(storage: naga::AddressSpace) -> TokenStream {
    match storage {
        naga::AddressSpace::Uniform => quote!(wgpu::BufferBindingType::Uniform),
        naga::AddressSpace::Storage { access } => {
            let _is_read = access.contains(naga::StorageAccess::LOAD);
            let is_write = access.contains(naga::StorageAccess::STORE);

            // TODO: Is this correct?
            if is_write {
                quote!(wgpu::BufferBindingType::Storage { read_only: false })
            } else {
                quote!(wgpu::BufferBindingType::Storage { read_only: true })
            }
        }
        _ => todo!(),
    }
}

pub fn vertex_format(ty: &naga::Type) -> wgpu::VertexFormat {
    // Not all wgsl types work as vertex attributes in wgpu.
    match &ty.inner {
        naga::TypeInner::Scalar(scalar) => match (scalar.kind, scalar.width) {
            (naga::ScalarKind::Sint, 4) => wgpu::VertexFormat::Sint32,
            (naga::ScalarKind::Uint, 4) => wgpu::VertexFormat::Uint32,
            (naga::ScalarKind::Float, 4) => wgpu::VertexFormat::Float32,
            (naga::ScalarKind::Float, 8) => wgpu::VertexFormat::Float64,
            _ => todo!(),
        },
        naga::TypeInner::Vector { size, scalar } => match size {
            naga::VectorSize::Bi => match (scalar.kind, scalar.width) {
                (naga::ScalarKind::Sint, 1) => wgpu::VertexFormat::Sint8x2,
                (naga::ScalarKind::Uint, 1) => wgpu::VertexFormat::Uint8x2,
                (naga::ScalarKind::Sint, 2) => wgpu::VertexFormat::Sint16x2,
                (naga::ScalarKind::Uint, 2) => wgpu::VertexFormat::Uint16x2,
                (naga::ScalarKind::Uint, 4) => wgpu::VertexFormat::Uint32x2,
                (naga::ScalarKind::Sint, 4) => wgpu::VertexFormat::Sint32x2,
                (naga::ScalarKind::Float, 4) => wgpu::VertexFormat::Float32x2,
                (naga::ScalarKind::Float, 8) => wgpu::VertexFormat::Float64x2,
                _ => todo!(),
            },
            naga::VectorSize::Tri => match (scalar.kind, scalar.width) {
                (naga::ScalarKind::Uint, 4) => wgpu::VertexFormat::Uint32x3,
                (naga::ScalarKind::Sint, 4) => wgpu::VertexFormat::Sint32x3,
                (naga::ScalarKind::Float, 4) => wgpu::VertexFormat::Float32x3,
                (naga::ScalarKind::Float, 8) => wgpu::VertexFormat::Float64x3,
                _ => todo!(),
            },
            naga::VectorSize::Quad => match (scalar.kind, scalar.width) {
                (naga::ScalarKind::Sint, 1) => wgpu::VertexFormat::Sint8x4,
                (naga::ScalarKind::Uint, 1) => wgpu::VertexFormat::Uint8x4,
                (naga::ScalarKind::Sint, 2) => wgpu::VertexFormat::Sint16x4,
                (naga::ScalarKind::Uint, 2) => wgpu::VertexFormat::Uint16x4,
                (naga::ScalarKind::Uint, 4) => wgpu::VertexFormat::Uint32x4,
                (naga::ScalarKind::Sint, 4) => wgpu::VertexFormat::Sint32x4,
                (naga::ScalarKind::Float, 4) => wgpu::VertexFormat::Float32x4,
                (naga::ScalarKind::Float, 8) => wgpu::VertexFormat::Float64x4,
                _ => todo!(),
            },
        },
        _ => todo!(), // are these types even valid as attributes?
    }
}

pub struct VertexInput {
    pub item_path: RustItemPath,
    pub fields: Vec<(u32, StructMember)>,
}

// TODO: Handle errors.
// Collect the necessary data to generate an equivalent Rust struct.
pub fn get_vertex_input_structs(
    invoking_entry_module: &str,
    module: &naga::Module,
) -> Vec<VertexInput> {
    // TODO: Handle multiple entries?
    module
        .entry_points
        .iter()
        .find(|e| e.stage == naga::ShaderStage::Vertex)
        .map(|vertex_entry| {
            vertex_entry
                .function
                .arguments
                .iter()
                .filter(|a| a.binding.is_none())
                .filter_map(|argument| {
                    let arg_type = &module.types[argument.ty];
                    match &arg_type.inner {
                        naga::TypeInner::Struct { members, span: _ } => {
                            let item_path = RustItemPath::new(
                                arg_type.name.as_ref().unwrap().into(),
                                invoking_entry_module.into(),
                            );

                            let input = VertexInput {
                                item_path,
                                fields: members
                                    .iter()
                                    .filter_map(|member| {
                                        // Skip builtins since they have no location binding.
                                        let location = match member.binding.as_ref().unwrap() {
                                            naga::Binding::BuiltIn(_) => None,
                                            naga::Binding::Location { location, .. } => {
                                                Some(*location)
                                            }
                                        }?;

                                        Some((location, member.clone()))
                                    })
                                    .collect(),
                            };

                            Some(input)
                        }

                        // An argument has to have a binding unless it is a structure.
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
