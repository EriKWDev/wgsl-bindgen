use std::collections::BTreeMap;

use derive_more::Constructor;
use generate::quote_shader_stages;
use quote::{format_ident, quote};
use quote_gen::rust_type;

use crate::wgsl::buffer_binding_type;
use crate::*;

mod entries_struct_builder;
use entries_struct_builder::*;

pub struct GroupData<'a> {
    pub bindings: Vec<GroupBinding<'a>>,
}

pub struct GroupBinding<'a> {
    pub name: Option<String>,
    pub binding_index: u32,
    pub binding_type: &'a naga::Type,
    pub address_space: naga::AddressSpace,
}

#[derive(Constructor)]
struct BindGroupBuilder<'a> {
    invoking_entry_name: &'a str,
    sanitized_entry_name: &'a str,
    group_no: u32,
    data: &'a GroupData<'a>,
    shader_stages: wgpu::ShaderStages,
    options: &'a WgslBindgenOption,
    naga_module: &'a naga::Module,
}

impl<'a> BindGroupBuilder<'a> {
    fn struct_name(&self) -> syn::Ident {
        self.options
            .wgpu_binding_generator
            .bind_group_layout
            .bind_group_name_ident(self.group_no)
    }

    fn bind_group_struct_impl(&self) -> TokenStream {
        // TODO: Support compute shader with vertex/fragment in the same module?
        let is_compute = self.shader_stages == wgpu::ShaderStages::COMPUTE;

        let render_pass = if is_compute {
            quote!(wgpu::ComputePass<'a>)
        } else {
            quote!(wgpu::RenderPass<'a>)
        };

        let bind_group_name = self.struct_name();
        let bind_group_entries_struct_name = self
            .options
            .wgpu_binding_generator
            .bind_group_layout
            .bind_group_entries_struct_name_ident(self.group_no);

        let entries: Vec<_> = self
            .data
            .bindings
            .iter()
            .map(|binding| {
                bind_group_layout_entry(
                    &self.invoking_entry_name,
                    self.naga_module,
                    self.options,
                    self.shader_stages,
                    binding,
                )
            })
            .collect();

        let names: Vec<_> = self
            .data
            .bindings
            .iter()
            .map(|binding| {
                // let name = demangle_str(binding.name.as_ref().unwrap());
                // let name = demangle_basic(binding.name.as_ref().unwrap());
                let name = binding.name.as_ref().unwrap();

                quote! {
                    #name
                }
            })
            .collect();

        let bind_group_layout_descriptor = {
            let bind_group_label = format!(
                "{}::BindGroup{}::LayoutDescriptor",
                self.sanitized_entry_name, self.group_no
            );
            quote! {
                wgpu::BindGroupLayoutDescriptor {
                    label: Some(#bind_group_label),
                    entries: Self::ENTRIES.as_slice(),
                }
            }
        };

        let group_no = Index::from(self.group_no as usize);
        let bind_group_label = format!("{}::BindGroup{}", self.sanitized_entry_name, self.group_no);

        let n = entries.len();

        quote! {
            impl #bind_group_name {
                pub const ENTRIES: [wgpu::BindGroupLayoutEntry; #n] = [
                    #(#entries),*
                ];
                pub const ENTRY_NAMES: [&'static str; #n] = [
                    #(#names),*
                ];

                pub const LAYOUT_DESCRIPTOR: wgpu::BindGroupLayoutDescriptor<'static> = #bind_group_layout_descriptor;

                #[inline(always)]
                pub fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
                    device.create_bind_group_layout(&Self::LAYOUT_DESCRIPTOR)
                }

                #[inline(always)]
                pub fn from_bindings(device: &wgpu::Device, bindings: #bind_group_entries_struct_name, layout: &wgpu::BindGroupLayout) -> Self {
                    let entries = bindings.as_array();
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(#bind_group_label),
                        entries: &entries,
                        layout,
                    });
                    Self(bind_group)
                }

                #[inline(always)]
                pub fn set<'a>(&'a self, render_pass: &mut #render_pass) {
                    render_pass.set_bind_group(#group_no, &self.0, &[]);
                }
            }
        }
    }

    fn build(self) -> TokenStream {
        let bind_group_name = self.struct_name();

        let group_struct = quote! {
            #[derive(Debug)]
            pub struct #bind_group_name(pub wgpu::BindGroup);
        };

        let group_impl = self.bind_group_struct_impl();

        quote! {
            #group_struct

            #group_impl
        }
    }
}

// TODO: Take an iterator instead?
pub fn bind_groups_module(
    invoking_entry_module: &str,
    options: &WgslBindgenOption,
    naga_module: &naga::Module,
    bind_group_data: &BTreeMap<u32, GroupData>,
    shader_stages: wgpu::ShaderStages,
) -> TokenStream {
    let sanitized_entry_name = sanitize_and_pascal_case(invoking_entry_module);

    let bind_groups: Vec<_> = bind_group_data
        .iter()
        .map(|(group_no, group)| {
            let wgpu_generator = &options.wgpu_binding_generator;

            let bind_group_entries_struct = BindGroupEntriesStructBuilder::new(
                invoking_entry_module,
                *group_no,
                group,
                &wgpu_generator.bind_group_layout,
            )
            .build();

            let additional_layout =
                if let Some(additional_generator) = &options.extra_binding_generator {
                    BindGroupEntriesStructBuilder::new(
                        invoking_entry_module,
                        *group_no,
                        group,
                        &additional_generator.bind_group_layout,
                    )
                    .build()
                } else {
                    quote!()
                };

            let bindgroup = BindGroupBuilder::new(
                &invoking_entry_module,
                &sanitized_entry_name,
                *group_no,
                group,
                shader_stages,
                options,
                naga_module,
            )
            .build();

            quote! {
              #additional_layout
              #bind_group_entries_struct
              #bindgroup
            }
        })
        .collect();

    let bind_group_fields: Vec<_> = bind_group_data
        .keys()
        .map(|group_no| {
            let group_name = options
                .wgpu_binding_generator
                .bind_group_layout
                .bind_group_name_ident(*group_no);
            let field = indexed_name_ident("bind_group", *group_no);
            quote!(pub #field: &'a #group_name)
        })
        .collect();

    // TODO: Support compute shader with vertex/fragment in the same module?
    let is_compute = shader_stages == wgpu::ShaderStages::COMPUTE;
    let render_pass = if is_compute {
        quote!(wgpu::ComputePass<'a>)
    } else {
        quote!(wgpu::RenderPass<'a>)
    };

    let group_parameters: Vec<_> = bind_group_data
        .keys()
        .map(|group_no| {
            let group = indexed_name_ident("bind_group", *group_no);
            let group_name = options
                .wgpu_binding_generator
                .bind_group_layout
                .bind_group_name_ident(*group_no);
            quote!(#group: &'a #group_name)
        })
        .collect();

    // The set function for each bind group already sets the index.
    let set_groups: Vec<_> = bind_group_data
        .keys()
        .map(|group_no| {
            let group = indexed_name_ident("bind_group", *group_no);
            quote!(#group.set(pass);)
        })
        .collect();

    let set_bind_groups = quote! {
        #[inline(always)]
        pub fn set_bind_groups<'a>(
            pass: &mut #render_pass,
            #(#group_parameters),*
        ) {
            #(#set_groups)*
        }
    };

    let all_bind_group_entries: Vec<_> = bind_group_data
        .keys()
        .map(|group_no| {
            let group_name = options
                .wgpu_binding_generator
                .bind_group_layout
                .bind_group_name_ident(*group_no);

            quote!(#group_name::ENTRIES)
        })
        .collect();

    let all_bind_group_entry_names: Vec<_> = bind_group_data
        .keys()
        .map(|group_no| {
            let group_name = options
                .wgpu_binding_generator
                .bind_group_layout
                .bind_group_name_ident(*group_no);

            quote!(#group_name::ENTRY_NAMES)
        })
        .collect();

    let num_groups = bind_groups.len();

    if bind_groups.is_empty() {
        // Don't include empty modules.
        quote!()
    } else {
        quote! {
          pub const NUM_BIND_GROUPS: usize = #num_groups;
          pub const BIND_GROUP_ENTRIES: [&'static [wgpu::BindGroupLayoutEntry]; NUM_BIND_GROUPS] = [
            #(#all_bind_group_entries.as_slice(),)*
          ];
          pub const BIND_GROUP_ENTRY_NAMES: [&'static [&'static str]; NUM_BIND_GROUPS] = [
            #(#all_bind_group_entry_names.as_slice(),)*
          ];

          pub fn extract_named_binding_types_for_all_bind_groups(output: &mut std::collections::HashMap<String, wgpu::BindingType>) {
            output.extend(BIND_GROUP_ENTRY_NAMES.into_iter().zip(BIND_GROUP_ENTRIES.into_iter()).flat_map(|(names, entries)| {
              assert!(names.len() == entries.len());
              names.into_iter().zip(entries.into_iter()).map(|(name, entry)| {
                (name.to_string(), entry.ty)
              })
            }));
          }

          pub fn construct_bind_group_values_from_named() -> Option<[; NUM_BIND_GROUPS]> {
          }

          #[derive(Debug, Copy, Clone)]
          pub struct WgpuBindGroups<'a> {
              #(#bind_group_fields),*
          }

          impl<'a> WgpuBindGroups<'a> {
              #[inline(always)]
              pub fn set(&self, pass: &mut #render_pass) {
                  #(self.#set_groups)*
              }
          }

          #set_bind_groups

          #(#bind_groups)*
        }
    }
}

fn bind_group_layout_entry(
    invoking_entry_module: &str,
    naga_module: &naga::Module,
    options: &WgslBindgenOption,
    shader_stages: wgpu::ShaderStages,
    binding: &GroupBinding,
) -> TokenStream {
    // TODO: Assume storage is only used for compute?
    // TODO: Support just vertex or fragment?
    // TODO: Visible from all stages?
    let stages = quote_shader_stages(shader_stages);

    let binding_index = Index::from(binding.binding_index as usize);
    // TODO: Support more types.
    let binding_type = match binding.binding_type.inner {
        naga::TypeInner::Scalar(_)
        | naga::TypeInner::Struct { .. }
        | naga::TypeInner::Array { .. } => {
            let buffer_binding_type = buffer_binding_type(binding.address_space);

            let rust_type = rust_type(
                Some(invoking_entry_module),
                naga_module,
                &binding.binding_type,
                options,
            );

            let min_binding_size = rust_type.quote_min_binding_size();

            quote!(wgpu::BindingType::Buffer {
                ty: #buffer_binding_type,
                has_dynamic_offset: false,
                min_binding_size: #min_binding_size,
            })
        }
        naga::TypeInner::Image { dim, class, .. } => {
            let view_dim = match dim {
                naga::ImageDimension::D1 => quote!(wgpu::TextureViewDimension::D1),
                naga::ImageDimension::D2 => quote!(wgpu::TextureViewDimension::D2),
                naga::ImageDimension::D3 => quote!(wgpu::TextureViewDimension::D3),
                naga::ImageDimension::Cube => quote!(wgpu::TextureViewDimension::Cube),
            };

            match class {
                naga::ImageClass::Sampled { kind, multi } => {
                    let sample_type = match kind {
                        naga::ScalarKind::Sint => quote!(wgpu::TextureSampleType::Sint),
                        naga::ScalarKind::Uint => quote!(wgpu::TextureSampleType::Uint),
                        naga::ScalarKind::Float => {
                            quote!(wgpu::TextureSampleType::Float { filterable: true })
                        }
                        _ => panic!("Unsupported sample type: {kind:#?}"),
                    };

                    // TODO: Don't assume all textures are filterable.
                    quote!(wgpu::BindingType::Texture {
                        sample_type: #sample_type,
                        view_dimension: #view_dim,
                        multisampled: #multi,
                    })
                }
                naga::ImageClass::Depth { multi } => {
                    quote!(wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: #view_dim,
                        multisampled: #multi,
                    })
                }
                naga::ImageClass::Storage { format, access } => {
                    // TODO: Will the debug implementation always work with the macro?
                    // Assume texture format variants are the same as storage formats.
                    let format = syn::Ident::new(&format!("{format:?}"), Span::call_site());
                    let storage_access = storage_access(access);

                    quote!(wgpu::BindingType::StorageTexture {
                        access: #storage_access,
                        format: wgpu::TextureFormat::#format,
                        view_dimension: #view_dim,
                    })
                }
            }
        }
        naga::TypeInner::Sampler { comparison } => {
            let sampler_type = if comparison {
                quote!(wgpu::SamplerBindingType::Comparison)
            } else {
                quote!(wgpu::SamplerBindingType::Filtering)
            };
            quote!(wgpu::BindingType::Sampler(#sampler_type))
        }
        // TODO: Better error handling.
        _ => panic!("Failed to generate BindingType."),
    };

    let doc = format!(
        " @binding({}): \"{}\"",
        binding.binding_index,
        binding.name.as_ref().unwrap(),
    );

    quote! {
        #[doc = #doc]
        wgpu::BindGroupLayoutEntry {
            binding: #binding_index,
            visibility: #stages,
            ty: #binding_type,
            count: None,
        }
    }
}

fn storage_access(access: naga::StorageAccess) -> TokenStream {
    let is_read = access.contains(naga::StorageAccess::LOAD);
    let is_write = access.contains(naga::StorageAccess::STORE);
    match (is_read, is_write) {
        (true, true) => quote!(wgpu::StorageTextureAccess::ReadWrite),
        (true, false) => quote!(wgpu::StorageTextureAccess::ReadOnly),
        (false, true) => quote!(wgpu::StorageTextureAccess::WriteOnly),
        _ => todo!(), // shouldn't be possible
    }
}

pub fn get_bind_group_data(
    module: &naga::Module,
) -> Result<BTreeMap<u32, GroupData>, CreateModuleError> {
    // Use a BTree to sort type and field names by group index.
    // This isn't strictly necessary but makes the generated code cleaner.
    let mut groups = BTreeMap::new();

    for global_handle in module.global_variables.iter() {
        let global = &module.global_variables[global_handle.0];
        if let Some(binding) = &global.binding {
            let group = groups.entry(binding.group).or_insert(GroupData {
                bindings: Vec::new(),
            });
            let binding_type = &module.types[module.global_variables[global_handle.0].ty];

            let group_binding = GroupBinding {
                name: global.name.clone(),
                binding_index: binding.binding,
                binding_type,
                address_space: global.space,
            };
            // Repeated bindings will probably cause a compile error.
            // We'll still check for it here just in case.
            if group
                .bindings
                .iter()
                .any(|g| g.binding_index == binding.binding)
            {
                return Err(CreateModuleError::DuplicateBinding {
                    binding: binding.binding,
                });
            }
            group.bindings.push(group_binding);
        }
    }

    // wgpu expects bind groups to be consecutive starting from 0.
    if groups.keys().map(|i| *i as usize).eq(0..groups.len()) {
        Ok(groups)
    } else {
        Err(CreateModuleError::NonConsecutiveBindGroups)
    }
}
