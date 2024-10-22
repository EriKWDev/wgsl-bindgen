use std::path::PathBuf;

use crate::{
    create_rust_bindings, WgslBindgenError, WgslBindgenOption, WgslEntryResult,
    WgslShaderIrCapabilities,
};

const PKG_VER: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub struct WGSLBindgen {
    pub options: WgslBindgenOption,
}

impl WGSLBindgen {
    pub fn new(options: WgslBindgenOption) -> Self {
        Self { options }
    }

    pub fn generate_naga_module_for_source<'a>(
        ir_capabilities: Option<WgslShaderIrCapabilities>,
        path: std::path::PathBuf,
        wgsl_source: String,
    ) -> Result<WgslEntryResult, WgslBindgenError> {
        let mut group_n = 0;
        let mut encountered = std::collections::HashMap::new();
        let wgsl_source = crate::fully_preprocess(
            wgsl_source,
            &mut std::collections::HashMap::default(),
            &mut Some(&mut |name| std::fs::read_to_string(path.parent().unwrap().join(name)).ok()),
            |key| {},
            |group| {
                let it = encountered.entry(group.to_string()).or_insert_with(|| {
                    let n = group_n;
                    group_n += 1;
                    (n, 0)
                });
                let res = *it;
                it.1 += 1;
                res
            },
        );

        let map_err = |err: naga::front::wgsl::ParseError| {
            let msg = err.emit_to_string(&wgsl_source);
            WgslBindgenError::NagaModuleComposeError {
                file_name: format!("{}", path.display()),
                inner: err,
                msg,
            }
        };

        let naga_module = naga::front::wgsl::parse_str(&wgsl_source).map_err(map_err)?;

        if let Some(cap) = ir_capabilities {
            let mut validator =
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), cap);

            if let Err(err) = validator.validate(&naga_module) {
                return Err(WgslBindgenError::NagaValidationError(err));
            }
        }

        let mut file_stem = path
            .file_stem()
            .map(|it| it.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("UNKNOWN"));

        if let Some((pre, _ext)) = file_stem.split_once(".") {
            file_stem = pre.into();
        }

        Ok(WgslEntryResult {
            naga_module,
            source_code: wgsl_source,
            module_name: file_stem,
        })
    }

    pub fn header_texts(&self) -> String {
        use std::fmt::Write;
        let mut text = String::new();

        let _ = writeln!(text, "");
        let _ = writeln!(text, "/* ");
        let _ = writeln!(
            text,
            "   NOTE: This file was automatically generated by {PKG_NAME} version {PKG_VER}"
        );
        let _ = writeln!(text, "");
        let _ = writeln!(
            text,
            "         Changes made to this file will not be saved."
        );
        let _ = writeln!(text, "*/");
        let _ = writeln!(text, "");

        text
    }

    pub fn generate_output<'a>(
        &self,
        all_shaders: impl IntoIterator<Item = (PathBuf, String)>,
    ) -> Result<String, WgslBindgenError> {
        let ir_capabilities = self.options.ir_capabilities;

        let entry_results = all_shaders
            .into_iter()
            .map(|(file_name, src)| {
                Self::generate_naga_module_for_source(ir_capabilities, file_name, src)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(create_rust_bindings(entry_results, &self.options)?)
    }
}
