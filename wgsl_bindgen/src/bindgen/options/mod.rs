mod bindings;
mod types;

use std::path::PathBuf;

pub use bindings::*;
use derive_builder::Builder;
pub use naga::valid::Capabilities as WgslShaderIrCapabilities;
use proc_macro2::TokenStream;
use regex::Regex;
pub use types::*;

use crate::{FastIndexMap, WGSLBindgen, WgslBindgenError, WgslType};

/// A struct representing a directory to scan for additional source files.
///
/// This struct is used to represent a directory to scan for additional source files
/// when generating Rust bindings for WGSL shaders. The `module_import_root` field
/// is used to specify the root prefix or namespace that should be applied to all
/// shaders given as the entrypoints, and the `directory` field is used to specify
/// the directory to scan for additional source files.
#[derive(Debug, Clone, Default)]
pub struct AdditionalScanDirectory {
    pub module_import_root: Option<String>,
    pub directory: String,
}

impl From<(Option<&str>, &str)> for AdditionalScanDirectory {
    fn from((module_import_root, directory): (Option<&str>, &str)) -> Self {
        Self {
            module_import_root: module_import_root.map(ToString::to_string),
            directory: directory.to_string(),
        }
    }
}

pub type WgslTypeMap = FastIndexMap<WgslType, TokenStream>;

/// A trait for building `WgslType` to `TokenStream` map.
///
/// This map is used to convert built-in WGSL types into their corresponding
/// representations in the generated Rust code. The specific format used for
/// matrix and vector types can vary, and the generated types for the same WGSL
/// type may differ in size or alignment.
///
/// Implementations of this trait provide a `build` function that takes a
/// `WgslTypeSerializeStrategy` and returns an `WgslTypeMap`.
pub trait WgslTypeMapBuild {
    /// Builds the `WgslTypeMap` based on the given serialization strategy.
    fn build(&self) -> WgslTypeMap;
}

impl WgslTypeMapBuild for WgslTypeMap {
    fn build(&self) -> WgslTypeMap {
        self.clone()
    }
}

/// This struct is used to create a custom mapping from the wgsl side to rust side,
/// skipping generation of the struct and using the custom one instead.
/// This also means skipping checks for alignment and size when using bytemuck
/// for the struct.
/// This is useful for core primitive types you would want to model in Rust side
#[derive(Clone, Debug)]
pub struct OverrideStruct {
    /// fully qualified struct name of the struct in wgsl, eg: `lib::fp64::Fp64`
    pub from: String,
    /// fully qualified struct name in your crate, eg: `crate::fp64::Fp64`
    pub to: TokenStream,
}

impl From<(&str, TokenStream)> for OverrideStruct {
    fn from((from, to): (&str, TokenStream)) -> Self {
        OverrideStruct {
            from: from.to_owned(),
            to,
        }
    }
}

/// Struct  for overriding the field type of specific structs.
#[derive(Clone, Debug)]
pub struct OverrideStructFieldType {
    pub struct_regex: Regex,
    pub field_regex: Regex,
    pub override_type: TokenStream,
}
impl From<(Regex, Regex, TokenStream)> for OverrideStructFieldType {
    fn from((struct_regex, field_regex, override_type): (Regex, Regex, TokenStream)) -> Self {
        Self {
            struct_regex,
            field_regex,
            override_type,
        }
    }
}
impl From<(&str, &str, TokenStream)> for OverrideStructFieldType {
    fn from((struct_regex, field_regex, override_type): (&str, &str, TokenStream)) -> Self {
        Self {
            struct_regex: Regex::new(struct_regex).expect("Failed to create struct regex"),
            field_regex: Regex::new(field_regex).expect("Failed to create field regex"),
            override_type,
        }
    }
}

/// Struct for overriding alignment of specific structs.
#[derive(Clone, Debug)]
pub struct OverrideStructAlignment {
    pub struct_regex: Regex,
    pub alignment: u16,
}
impl From<(Regex, u16)> for OverrideStructAlignment {
    fn from((struct_regex, alignment): (Regex, u16)) -> Self {
        Self {
            struct_regex,
            alignment,
        }
    }
}
impl From<(&str, u16)> for OverrideStructAlignment {
    fn from((struct_regex, alignment): (&str, u16)) -> Self {
        Self {
            struct_regex: Regex::new(struct_regex).expect("Failed to create struct regex"),
            alignment,
        }
    }
}

#[derive(Debug, Default, Builder)]
#[builder(
    setter(into),
    field(private),
    build_fn(private, name = "fallible_build")
)]
pub struct WgslBindgenOption {
    /// Derive [serde::Serialize](https://docs.rs/serde/1.0.159/serde/trait.Serialize.html)
    /// and [serde::Deserialize](https://docs.rs/serde/1.0.159/serde/trait.Deserialize.html)
    /// for user defined WGSL structs when `true`.
    #[builder(default = "false")]
    pub derive_serde: bool,

    /// The [wgpu::naga::valid::Capabilities](https://docs.rs/wgpu/latest/wgpu/naga/valid/struct.Capabilities.html) to support. Defaults to `None`.
    #[builder(default, setter(strip_option))]
    pub ir_capabilities: Option<WgslShaderIrCapabilities>,

    /// A mapping operation for WGSL built-in types. This is used to map WGSL built-in types to their corresponding representations.
    #[builder(setter(custom))]
    pub type_map: WgslTypeMap,

    /// A vector of custom struct mappings to be added, which will override the struct to be generated.
    /// This is merged with the default struct mappings.
    #[builder(default, setter(each(name = "add_override_struct_mapping", into)))]
    pub override_struct: Vec<OverrideStruct>,

    /// A vector of `OverrideStructFieldType` to override the generated types for struct fields in matching structs.
    #[builder(default, setter(into))]
    pub override_struct_field_type: Vec<OverrideStructFieldType>,

    /// A vector of regular expressions and alignments that override the generated alignment for matching structs.
    /// This can be used in scenarios where a specific minimum alignment is required for a uniform buffer.
    /// Refer to the [WebGPU specs](https://www.w3.org/TR/webgpu/#dom-supported-limits-minuniformbufferoffsetalignment) for more information.
    #[builder(default, setter(into))]
    pub override_struct_alignment: Vec<OverrideStructAlignment>,

    /// The regular expression of the padding fields used in the shader struct types.
    /// These fields will be omitted in the *Init structs generated, and will automatically be assigned the default values.
    #[builder(default, setter(each(name = "add_custom_padding_field_regexp", into)))]
    pub custom_padding_field_regexps: Vec<Regex>,

    /// Whether to always have the init struct generated in the out. This is only applicable when using bytemuck mode.
    #[builder(default = "false")]
    pub always_generate_init_struct: bool,

    /// Whether to implement Zeroable for init structs as well. This is only applicable when using bytemuck mode.
    #[builder(default = "false")]
    pub impl_zeroable_for_init_structs: bool,

    /// This field can be used to provide a custom generator for extra bindings that are not covered by the default generator.
    #[builder(default, setter(custom))]
    pub extra_binding_generator: Option<BindingGenerator>,

    /// This field is used to provide the default generator for WGPU bindings. The generator is represented as a `BindingGenerator`.
    #[builder(default, setter(custom))]
    pub wgpu_binding_generator: BindingGenerator,
}

impl WgslBindgenOptionBuilder {
    pub fn build(&mut self) -> Result<WGSLBindgen, WgslBindgenError> {
        self.merge_struct_type_overrides();
        let options = self.fallible_build()?;
        Ok(WGSLBindgen::new(options))
    }

    pub fn type_map(&mut self, map_build: impl WgslTypeMapBuild) -> &mut Self {
        let map = map_build.build();

        match self.type_map.as_mut() {
            Some(m) => m.extend(map),
            None => self.type_map = Some(map),
        }

        self
    }

    fn merge_struct_type_overrides(&mut self) {
        let struct_mappings = self
            .override_struct
            .iter()
            .flatten()
            .map(|mapping| {
                let wgsl_type = WgslType::Struct {
                    fully_qualified_name: mapping.from.clone(),
                };
                (wgsl_type, mapping.to.clone())
            })
            .collect::<FastIndexMap<_, _>>();

        self.type_map(struct_mappings);
    }

    pub fn extra_binding_generator(
        &mut self,
        config: impl GetBindingsGeneratorConfig,
    ) -> &mut Self {
        let generator = Some(config.get_generator_config());
        self.extra_binding_generator = Some(generator);
        self
    }
}
