use super::{Engine, Value, ty};
use miren::replay::{ReplayArgument, ReplayArguments, ReplayValue};
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use std::collections::BTreeMap;

impl<'tcx> Engine<'tcx> {
    pub(super) fn replay_arguments(&self, id: DefId, values: &[Value]) -> ReplayArguments {
        let build = || {
            if self.tcx.def_kind(id) != DefKind::Fn
                || !self.tcx.visibility(id).is_public()
                || self.tcx.generics_of(id).parent_count != 0
                || !self.tcx.generics_of(id).own_params.is_empty()
            {
                return Err("replay requires a public, nongeneric free function".to_owned());
            }
            let mut parent = self.tcx.opt_parent(id);
            while let Some(module) = parent {
                if self.tcx.def_kind(module) != DefKind::Mod {
                    return Err("replay requires a free function outside impl blocks".to_owned());
                }
                parent = self.tcx.opt_parent(module);
                if parent.is_some() && !self.tcx.visibility(module).is_public() {
                    return Err("replay function is inside an inaccessible module".to_owned());
                }
            }
            let body = self.tcx.optimized_mir(id);
            body.args_iter()
                .zip(values)
                .map(|(local, value)| {
                    let ty = body.local_decls[local].ty;
                    let name = body
                        .var_debug_info
                        .iter()
                        .find_map(|info| match info.value {
                            rustc_middle::mir::VarDebugInfoContents::Place(place)
                                if place.local == local && place.projection.is_empty() =>
                            {
                                Some(info.name.to_string())
                            }
                            rustc_middle::mir::VarDebugInfoContents::Place(_)
                            | rustc_middle::mir::VarDebugInfoContents::Const(_) => None,
                        })
                        .unwrap_or_else(|| format!("arg{}", local.as_usize() - 1));
                    Ok(ReplayArgument {
                        name,
                        rust_type: format!("{ty}"),
                        value: Self::replay_value(ty, value)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        let source_files = self
            .tcx
            .sess
            .source_map()
            .files()
            .iter()
            .filter_map(|source| {
                if source.cnum != rustc_span::def_id::LOCAL_CRATE {
                    return None;
                }
                let text = source.src.as_ref()?;
                let path = source.name.prefer_local_unconditionally().to_string();
                let path = std::fs::canonicalize(path)
                    .ok()?
                    .to_string_lossy()
                    .into_owned();
                Some((path, miren::replay::source_fingerprint(text)))
            })
            .collect::<BTreeMap<_, _>>();
        let working_directory = std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
        let build_environment = [
            "CARGO_CRATE_NAME",
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_BIN_NAME",
            "CARGO_PKG_NAME",
            "CARGO_PKG_VERSION",
            "CARGO_PKG_VERSION_MAJOR",
            "CARGO_PKG_VERSION_MINOR",
            "CARGO_PKG_VERSION_PATCH",
            "CARGO_PKG_VERSION_PRE",
            "CARGO_PKG_AUTHORS",
            "CARGO_PKG_DESCRIPTION",
            "CARGO_PKG_HOMEPAGE",
            "CARGO_PKG_REPOSITORY",
            "CARGO_PKG_LICENSE",
            "CARGO_PKG_LICENSE_FILE",
            "CARGO_PKG_RUST_VERSION",
            "OUT_DIR",
        ]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_owned(), value))
        })
        .collect::<BTreeMap<_, _>>();
        match build() {
            Ok(arguments) => ReplayArguments {
                arguments,
                unsupported: None,
                source_files,
                working_directory,
                build_environment,
            },
            Err(reason) => ReplayArguments {
                arguments: Vec::new(),
                unsupported: Some(reason),
                source_files,
                working_directory,
                build_environment,
            },
        }
    }

    fn replay_value(ty: ty::Ty<'tcx>, value: &Value) -> Result<ReplayValue, String> {
        let value = value.materialize()?;
        let symbol = |expression: &super::Term| {
            expression
                .symbol_index()
                .map(|index| format!("v{index}"))
                .ok_or_else(|| "replay input is not a direct solver symbol".to_owned())
        };
        match (ty.kind(), &value) {
            (ty::Bool, Value::Bool(expression)) => Ok(ReplayValue::Bool {
                symbol: symbol(expression)?,
            }),
            (
                ty::Int(_) | ty::Uint(_),
                Value::Int {
                    expression,
                    bits,
                    signed,
                },
            ) => Ok(ReplayValue::Integer {
                symbol: symbol(expression)?,
                bits: *bits,
                signed: *signed,
            }),
            (ty::Tuple(types), Value::Unit) if types.is_empty() => Ok(ReplayValue::Unit),
            (ty::Array(element, _), Value::Elements(elements)) if elements.len() <= 128 => {
                Ok(ReplayValue::Array {
                    elements: elements
                        .iter()
                        .map(|value| Self::replay_value(*element, value))
                        .collect::<Result<_, _>>()?,
                })
            }
            _ => Err(format!("native replay input type {ty} is not supported")),
        }
    }
}
