use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Ident;

use crate::quote_gen::{rust_type, RustItem, RustItemPath, RustItemType};
use crate::WgslBindgenOption;

pub fn consts_items(invoking_entry_module: &str, module: &naga::Module) -> Vec<RustItem> {
    // Create matching Rust constants for WGSl constants.
    module
        .constants
        .iter()
        .filter_map(|(_, t)| -> Option<RustItem> {
            let name_str = t.name.as_ref()?;

            // we don't need full qualification here
            let rust_item_path = RustItemPath::new(name_str.into(), invoking_entry_module.into());
            let name = Ident::new(&rust_item_path.name, Span::call_site());

            // TODO: Add support for f64 and f16 once naga supports them.
            let type_and_value = match &module.global_expressions[t.init] {
                naga::Expression::Literal(literal) => match literal {
                    naga::Literal::F64(v) => Some(quote!(f32 = #v)),
                    naga::Literal::F32(v) => Some(quote!(f32 = #v)),
                    naga::Literal::U32(v) => Some(quote!(u32 = #v)),
                    naga::Literal::U64(v) => Some(quote!(u64 = #v)),
                    naga::Literal::I32(v) => Some(quote!(i32 = #v)),
                    naga::Literal::Bool(v) => Some(quote!(bool = #v)),
                    naga::Literal::I64(v) => Some(quote!(i64 = #v)),
                    naga::Literal::AbstractInt(v) => Some(quote!(i64 = #v)),
                    naga::Literal::AbstractFloat(v) => Some(quote!(f64 = #v)),
                },
                _ => None,
            }?;

            Some(RustItem::new(
                RustItemType::ConstVarDecls.into(),
                rust_item_path,
                quote! { pub const #name: #type_and_value;},
            ))
        })
        .collect()
}

pub fn pipeline_overridable_constants(
    module: &naga::Module,
    options: &WgslBindgenOption,
) -> TokenStream {
    let overrides: Vec<_> = module.overrides.iter().map(|(_, o)| o).collect();

    let fields: Vec<_> = overrides
        .iter()
        .map(|o| {
            let name = Ident::new(o.name.as_ref().unwrap(), Span::call_site());
            // TODO: Do we only need to handle scalar types here?
            let ty = rust_type(None, module, &module.types[o.ty], options);

            if o.init.is_some() {
                quote!(pub #name: Option<#ty>)
            } else {
                quote!(pub #name: #ty)
            }
        })
        .collect();

    let required_entries: Vec<_> = overrides
      .iter()
      .filter_map(|o| {
          if o.init.is_some() {
              None
          } else {
              let key = override_key(o);

              let name = Ident::new(o.name.as_ref().unwrap(), Span::call_site());

              // TODO: Do we only need to handle scalar types here?
              let ty = &module.types[o.ty];
              let value = if matches!(ty.inner, naga::TypeInner::Scalar(s) if s.kind == naga::ScalarKind::Bool) {
                  quote!(if self.#name { 1.0 } else { 0.0})
              } else {
                  quote!(self.#name as f64)
              };

              Some(quote!((#key.to_owned(), #value)))
          }
      })
      .collect();

    // Add code for optionally inserting the constants with defaults.
    // Omitted constants will be initialized using the values defined in WGSL.
    let insert_optional_entries: Vec<_> = overrides
      .iter()
      .filter_map(|o| {
          if o.init.is_some() {
              let key = override_key(o);

              // TODO: Do we only need to handle scalar types here?
              let ty = &module.types[o.ty];
              let value = if matches!(ty.inner, naga::TypeInner::Scalar(s) if s.kind == naga::ScalarKind::Bool) {
                  quote!(if value { 1.0 } else { 0.0})
              } else {
                  quote!(value as f64)
              };

              let name = Ident::new(o.name.as_ref().unwrap(), Span::call_site());

              Some(quote! {
                  if let Some(value) = self.#name {
                      entries.insert(#key.to_owned(), #value);
                  }
              })
          } else {
              None
          }
      })
      .collect();

    let init_entries = if insert_optional_entries.is_empty() {
        quote!(let entries = std::collections::HashMap::from([#(#required_entries),*]);)
    } else {
        quote!(let mut entries = std::collections::HashMap::from([#(#required_entries),*]);)
    };

    if !fields.is_empty() {
        // Create a Rust struct that can initialize the constants dictionary.
        quote! {
            pub struct OverrideConstants {
                #(#fields),*
            }

            // TODO: Only start with the required ones.
            impl OverrideConstants {
                pub fn constants(&self) -> std::collections::HashMap<String, f64> {
                    #init_entries
                    #(#insert_optional_entries);*
                    entries
                }
            }
        }
    } else {
        quote!()
    }
}

fn override_key(o: &naga::Override) -> String {
    // The @id(id) should be the name if present.
    o.id.map(|i| i.to_string())
        .unwrap_or(o.name.clone().unwrap())
}
