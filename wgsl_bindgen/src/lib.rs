#[allow(dead_code, unused)]
extern crate wgpu_types as wgpu;

use case::CaseExt;
use generate::entry::{self, entry_point_constants, vertex_struct_impls};
use generate::{bind_group, consts, pipeline};
use heck::ToPascalCase;
use proc_macro2::{Span, TokenStream};
use qs::{format_ident, quote, Ident, Index};
use quote_gen::custom_vector_matrix_assertions;
use thiserror::Error;

mod bindgen;
mod generate;
mod quote_gen;
mod structs;
mod types;
mod wgsl;
mod wgsl_type;

pub mod qs {
    pub use proc_macro2::TokenStream;
    pub use quote::{format_ident, quote};
    pub use syn::{Ident, Index};
}

pub use bindgen::*;
pub use naga::FastIndexMap;
pub use regex::Regex;
pub use types::*;
pub use wgsl_type::*;

/// Errors while generating Rust source for a WGSl shader module.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum CreateModuleError {
    /// Bind group sets must be consecutive and start from 0.
    /// See `bind_group_layouts` for
    /// [PipelineLayoutDescriptor](https://docs.rs/wgpu/latest/wgpu/struct.PipelineLayoutDescriptor.html#).
    #[error("bind groups are non-consecutive or do not start from 0")]
    NonConsecutiveBindGroups,

    /// Each binding resource must be associated with exactly one binding index.
    #[error("duplicate binding found with index `{binding}`")]
    DuplicateBinding { binding: u32 },
}

#[derive(Debug)]
pub struct WgslEntryResult {
    pub naga_module: naga::Module,
    pub source_code: String,
    pub module_name: String,
}

fn create_rust_bindings(
    entries: Vec<WgslEntryResult>,
    options: &WgslBindgenOption,
) -> Result<String, CreateModuleError> {
    let math_asserts = custom_vector_matrix_assertions(options);

    let mut parts = vec![];
    let mut mod_names = vec![];

    for entry in entries.iter() {
        let WgslEntryResult {
            naga_module,
            module_name,
            ..
        } = entry;

        let mod_name = module_name;

        let entry_name = sanitize_and_pascal_case(&mod_name);
        let bind_group_data = bind_group::get_bind_group_data(naga_module)?;
        let shader_stages = wgsl::shader_stages(naga_module);

        // Write all the structs, including uniforms and entry function inputs.
        let structs = structs::structs_items(naga_module, options);
        let consts = consts::consts_items(naga_module);
        let overridable_consts = consts::pipeline_overridable_constants(naga_module, options);
        let vertex_struct_impls = vertex_struct_impls(mod_name, naga_module);

        let bind_groups = bind_group::bind_groups_module(
            &mod_name,
            &options,
            naga_module,
            &bind_group_data,
            shader_stages,
        );

        let entry_point_constants = entry_point_constants(naga_module);

        // mod_builder.add(
        //     mod_name,
        //     shader_module::compute_module(naga_module, options.shader_source_type),
        // );

        let vertex_states_entry = entry::vertex_states(mod_name, naga_module);
        let fragment_states_entry = entry::fragment_states(naga_module);

        let create_pipeline_layout = pipeline::create_pipeline_layout_fn(
            &entry_name,
            naga_module,
            shader_stages,
            &options,
            &bind_group_data,
        );

        let all = vec![
            vertex_struct_impls
                .into_iter()
                .map(|it| it.item)
                .collect::<Vec<_>>(),
            structs.into_iter().map(|it| it.item).collect::<Vec<_>>(),
        ];

        let mod_name = qs::format_ident!("{mod_name}");

        parts.push(quote! {
            pub mod #mod_name {
                use super::*;

                #consts
                #bind_groups

                #(#(#all)*)*
            }
        });
        mod_names.push(mod_name);
    }

    let output = quote! {
      #![allow(unused, non_snake_case, non_camel_case_types, non_upper_case_globals)]

      pub fn extract_named_bind_group_variables_and_types_for_all_shaders(output: &mut std::collections::HashMap<String, wgpu::BindGroupLayoutEntry>) {
          #(#mod_names::extract_named_binding_types_for_all_bind_groups(output);)*
      }

      #(#parts)*

      pub mod math_asserts {
          #math_asserts
      }
    };

    Ok(pretty_print(&output))
}

fn pretty_print(tokens: &TokenStream) -> String {
    let it = tokens.to_string();

    if std::env::var("WGSL_DEBUG").is_ok() {
        let _ = std::fs::write("output.rs", &it);
    }

    let file = syn::parse_file(&it).unwrap();
    prettyplease::unparse(&file)
}

fn indexed_name_ident(name: &str, index: u32) -> Ident {
    format_ident!("{name}{index}")
}

fn sanitize_and_pascal_case(v: &str) -> String {
    v.chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_pascal_case()
}

fn sanitized_upper_snake_case(v: &str) -> String {
    v.chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_snake()
        .to_uppercase()
}

use std::{collections::HashMap, fmt::Write, io::BufRead};

/// Resolves '#import':s and '#ifdef':s and more
///
/// use `preprocess_resolve_imports` and `preprocess_resolve_ifdefs` separately for more efficient strategies
pub fn fully_preprocess(
    src: impl AsRef<str>,
    defs: &mut HashMap<String, String>,

    get_source_of_file_name: &mut Option<&mut impl FnMut(String) -> Option<String>>,
    encountered_ifdef: impl FnMut(&str),
    group_and_binding_for: impl FnMut(&str) -> (usize, usize),
) -> String {
    let src = src.as_ref();
    let resolved = preprocess_resolve_imports(std::io::Cursor::new(src), get_source_of_file_name);
    preprocess_resolve_ifdefs(
        std::io::Cursor::new(resolved),
        defs,
        encountered_ifdef,
        group_and_binding_for,
    )
}

/// resolves #import "file.extension" statements recursively for the whole file
pub fn preprocess_resolve_imports<F: FnMut(String) -> Option<String>>(
    src: impl BufRead,
    get_source_of_file_name: &mut Option<&mut F>,
) -> String {
    const MAX_RECURSION: usize = 100;
    preprocess_resolve_imports_recursively::<_, MAX_RECURSION>(src, get_source_of_file_name, 0)
}

/// Use after #import have been resolved using `preprocess_wgsl_source_resolve_imports`
///
/// handles #ifdef, #ifndef, #define, #endif
pub fn preprocess_resolve_ifdefs(
    src: impl BufRead,
    defs: &mut HashMap<String, String>,

    encountered_ifdef: impl FnMut(&str),
    group_and_binding_for: impl FnMut(&str) -> (usize, usize),
) -> String {
    let (resulting_depth, res) = preprocess_resolve_ifdefs_implementation(
        src,
        defs,
        encountered_ifdef,
        group_and_binding_for,
        0,
    );
    if resulting_depth != 0 {
        println!(
            "Resulting depth after preprocessing source was not back at top. This would suggest a potentially unclosed '#ifdef'"
        )
    }
    res
}

pub fn preprocess_resolve_imports_recursively<
    F: FnMut(String) -> Option<String>,
    const MAX_RECURSION: usize,
>(
    src: impl BufRead,
    get_source_of_file_name: &mut Option<&mut F>,
    depth: usize,
) -> String {
    let mut output = String::new();
    let mut lines = src.lines();

    if depth > MAX_RECURSION {
        println!("Recursion depth hit max during '#import' resolving (depth: {depth} > max: {MAX_RECURSION})");
        return format!("");
    }

    while let Some(Ok(line)) = lines.next() {
        let line_trimmed = line.trim();
        if line_trimmed.starts_with('#') {
            if let Some((_s, rest)) = line_trimmed.split_once('#') {
                let mut rest = rest.split_whitespace();

                if let Some(op) = rest.next() {
                    match op {
                        "import" | "include" => {
                            if let Some(get_shader_source) = get_source_of_file_name {
                                if let Some(mut shader_name) = rest.next() {
                                    if let Some(new) = shader_name.strip_prefix('"') {
                                        shader_name = new;
                                    }
                                    if let Some(new) = shader_name.strip_suffix('"') {
                                        shader_name = new;
                                    }
                                    if let Some(new) = shader_name.strip_prefix('<') {
                                        shader_name = new;
                                    }
                                    if let Some(new) = shader_name.strip_suffix('>') {
                                        shader_name = new;
                                    }

                                    if let Some(source_to_paste) =
                                        get_shader_source(shader_name.into())
                                    {
                                        let source_to_paste =
                                            preprocess_resolve_imports_recursively::<
                                                F,
                                                MAX_RECURSION,
                                            >(
                                                std::io::Cursor::new(source_to_paste),
                                                get_source_of_file_name,
                                                depth + 1,
                                            );

                                        let _ = output.write_fmt(format_args!(
                                            "// {line_trimmed} // BEGIN #import '{shader_name}'\n"
                                        ));
                                        output.push_str(&source_to_paste);
                                        let _ = output.write_fmt(format_args!(
                                            "// END #import '{shader_name}'\n"
                                        ));
                                        continue;
                                    }
                                }
                            }

                            let _ = output.write_fmt(format_args!(
                                "// {line_trimmed} // ERROR: Preprocessor could not include file, skipped\n"
                            ));
                            continue;
                        }

                        _ => {}
                    }
                }
            }
        }
        output.push_str(&line);
        output.push('\n');
    }

    output
}

/// implementation of `preprocess_ifdefs`
pub fn preprocess_resolve_ifdefs_implementation(
    src: impl BufRead,
    defs: &mut HashMap<String, String>,

    mut encountered_ifdef: impl FnMut(&str),
    mut group_and_binding_for: impl FnMut(&str) -> (usize, usize),

    start_depth: usize,
) -> (usize, String) {
    let mut output = String::new();

    let mut lines = src.lines();

    let mut depth = start_depth;
    let mut should_skip = depth != 0;

    while let Some(Ok(mut line)) = lines.next() {
        let line_trimmed = line.trim();

        let mut comment_this_line = should_skip;
        let mut reason = None;

        if line_trimmed.starts_with('#') {
            if let Some((_s, rest)) = line_trimmed.split_once('#') {
                let mut rest = rest.split_whitespace();

                if let Some(op) = rest.next() {
                    if should_skip {
                        match op {
                            "endif" => {
                                if depth > 0 {
                                    depth = depth - 1;
                                }
                            }

                            "ifdef" => {
                                depth = depth + 1;
                            }

                            "else" => {
                                if depth > 0 {
                                    depth = depth - 1;
                                    reason = Some(format!(" (true)"));
                                } else {
                                    reason = Some(format!(" (false)"));
                                }
                            }

                            _ => {}
                        }
                    } else {
                        match op {
                            "endif" => {
                                if depth > 0 {
                                    depth = depth - 1;
                                }
                                comment_this_line = true;
                            }

                            "ifdef" => {
                                if let Some(key) = rest.next() {
                                    encountered_ifdef(key);
                                    if !defs.contains_key(key) {
                                        depth += 1;
                                        should_skip = true;
                                        reason = Some(format!(" (false)"))
                                    } else {
                                        reason = Some(format!(" (true)"))
                                    }
                                }
                                comment_this_line = true;
                            }

                            "else" => {
                                if depth > 0 {
                                    reason = Some(format!(" (true)"));
                                    depth = depth - 1;
                                } else {
                                    reason = Some(format!(" (false)"));
                                }
                                comment_this_line = true;
                            }

                            "define" => {
                                if let (Some(key), val) = (rest.next(), rest.next()) {
                                    let key = key.trim();
                                    reason = Some(format!(" (defined {key})"));
                                    defs.insert(
                                        key.into(),
                                        val.unwrap_or_default().trim().to_string(),
                                    );
                                }
                                comment_this_line = true;
                            }

                            "undef" | "undefine" => {
                                if let Some(key) = rest.next() {
                                    let key = key.trim();
                                    reason = Some(format!(" (undefined {key})"));
                                    defs.remove(key);
                                }
                                comment_this_line = true;
                            }

                            "import" => {
                                panic!(
                                    "imports should have already been resolved by a previous pass"
                                )
                            }

                            _ => {}
                        }
                    }
                }
            }
        }

        if line_trimmed.starts_with('@') {
            if let Some((_s, rest)) = line_trimmed.split_once('@') {
                if let Some((_s, rest)) = rest.split_once("group_binding") {
                    if let Some((_s, rest)) = rest.split_once('(') {
                        if let Some((name, _rest)) = rest.split_once(')') {
                            let name = name.trim();
                            let (group, binding) = group_and_binding_for(&name);
                            line = line.replace(
                                &format!("@group_binding({name})"),
                                &format!("@group({group}) @binding({binding})"),
                            );
                        }
                    }
                }
            }
        }

        if comment_this_line {
            let _ = output.write_fmt(format_args!("//\t\t{line}",));
            if let Some(reason) = reason {
                let _ = output.write_fmt(format_args!("\t{reason}",));
            }
            output.push('\n');
        } else {
            let mut replaced_line = line.clone();
            for (key, value) in defs.iter() {
                replaced_line = replaced_line.replace(key, &value);
            }
            let _ = output.write_fmt(format_args!("{replaced_line}\n"));
        }

        let stop_skipping = should_skip && depth == 0;
        if stop_skipping {
            should_skip = false;
        }
    }

    (depth, output)
}
