use std::collections::HashSet;

use naga::{Handle, Type};

use crate::quote_gen::{RustItem, RustItemPath, RustStructBuilder};
use crate::WgslBindgenOption;

pub fn structs_items(module: &naga::Module, options: &WgslBindgenOption) -> Vec<RustItem> {
    // Initialize the layout calculator provided by naga.
    let mut layouter = naga::proc::Layouter::default();
    layouter.update(module.to_ctx()).unwrap();

    let mut global_variable_types = HashSet::new();
    for g in module.global_variables.iter() {
        add_types_recursive(&mut global_variable_types, module, g.1.ty);
    }

    // Create matching Rust structs for WGSL structs.
    // This is a UniqueArena, so each struct will only be generated once.
    module
        .types
        .iter()
        .filter(|(h, _)| {
            // Check if the struct will need to be used by the user from Rust.
            // This includes function inputs like vertex attributes and global variables.
            // Shader stage function outputs will not be accessible from Rust.
            // Skipping internal structs helps avoid issues deriving encase or bytemuck.
            !module
                .entry_points
                .iter()
                .any(|e| e.function.result.as_ref().map(|r| r.ty) == Some(*h))
                && module
                    .entry_points
                    .iter()
                    .any(|e| e.function.arguments.iter().any(|a| a.ty == *h))
                || global_variable_types.contains(h)
        })
        .flat_map(|(t_handle, ty)| {
            if let naga::TypeInner::Struct { members, .. } = &ty.inner {
                let rust_item_path = RustItemPath::new("".into(), ty.name.as_ref().unwrap().into());

                // skip if using custom struct mapping
                if options.type_map.contains_key(&crate::WgslType::Struct {
                    fully_qualified_name: rust_item_path.get_fully_qualified_name().into(),
                }) {
                    Vec::new()
                } else {
                    rust_struct(
                        &rust_item_path,
                        members,
                        &layouter,
                        t_handle,
                        module,
                        options,
                        &global_variable_types,
                    )
                }
            } else {
                Vec::new()
            }
        })
        .collect()
}

fn rust_struct(
    rust_item_path: &RustItemPath,
    naga_members: &[naga::StructMember],
    layouter: &naga::proc::Layouter,
    t_handle: naga::Handle<naga::Type>,
    naga_module: &naga::Module,
    options: &WgslBindgenOption,
    global_variable_types: &HashSet<Handle<Type>>,
) -> Vec<RustItem> {
    let layout = layouter[t_handle];

    // Assume types used in global variables are host shareable and require validation.
    // This includes storage, uniform, and workgroup variables.
    // This also means types that are never used will not be validated.
    // Structs used only for vertex inputs do not require validation on desktop platforms.
    // Vertex input layout is handled already by setting the attribute offsets and types.
    // This allows vertex input field types without padding like vec3 for positions.
    let is_host_sharable = global_variable_types.contains(&t_handle);

    let has_rts_array = struct_has_rts_array_member(naga_members, naga_module);
    let is_directly_sharable = is_host_sharable;

    let builder = RustStructBuilder::from_naga(
        rust_item_path,
        naga_members,
        naga_module,
        &options,
        layout,
        is_directly_sharable,
        is_host_sharable,
        has_rts_array,
    );
    builder.build()
}

fn add_types_recursive(
    types: &mut HashSet<naga::Handle<naga::Type>>,
    module: &naga::Module,
    ty: Handle<Type>,
) {
    types.insert(ty);

    match &module.types[ty].inner {
        naga::TypeInner::Pointer { base, .. } => add_types_recursive(types, module, *base),
        naga::TypeInner::Array { base, .. } => add_types_recursive(types, module, *base),
        naga::TypeInner::Struct { members, .. } => {
            for member in members {
                add_types_recursive(types, module, member.ty);
            }
        }
        naga::TypeInner::BindingArray { base, .. } => add_types_recursive(types, module, *base),
        _ => (),
    }
}

fn struct_has_rts_array_member(members: &[naga::StructMember], module: &naga::Module) -> bool {
    members.iter().any(|m| {
        matches!(
            module.types[m.ty].inner,
            naga::TypeInner::Array {
                size: naga::ArraySize::Dynamic,
                ..
            }
        )
    })
}
