use codemap::{CodeMap, Span};
use serde::{Deserialize, Serialize};

use core_model_builder::{
    ast::ast_types::{AstAnnotationParams, AstExpr},
    typechecker::Typed,
};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ResolvedAccess {
    pub creation: AstExpr<Typed>,
    pub read: AstExpr<Typed>,
    pub update: AstExpr<Typed>,
    pub delete: AstExpr<Typed>,
}

impl ResolvedAccess {
    fn permissive() -> Self {
        ResolvedAccess {
            creation: AstExpr::BooleanLiteral(true, null_span()),
            read: AstExpr::BooleanLiteral(true, null_span()),
            update: AstExpr::BooleanLiteral(true, null_span()),
            delete: AstExpr::BooleanLiteral(true, null_span()),
        }
    }
}

fn null_span() -> Span {
    let mut codemap = CodeMap::new();
    let file = codemap.add_file("".to_string(), "".to_string());
    file.span
}

pub fn build_access(
    access_annotation_params: Option<&AstAnnotationParams<Typed>>,
) -> ResolvedAccess {
    match access_annotation_params {
        Some(p) => {
            let restrictive = AstExpr::BooleanLiteral(false, null_span());

            // The annotation parameter hierarchy is:
            // value -> query
            //       -> mutation -> create
            //                   -> update
            //                   -> delete
            // Any lower node in the hierarchy get a priority over its parent.

            let (creation, read, update, delete) = match p {
                AstAnnotationParams::Single(default, _) => (default, default, default, default),
                AstAnnotationParams::Map(m, _) => {
                    let query = m.get("query");
                    let mutation = m.get("mutation");
                    let create = m.get("create");
                    let update = m.get("update");
                    let delete = m.get("delete");

                    let default_mutation = mutation.unwrap_or(&restrictive);

                    (
                        create.unwrap_or(default_mutation),
                        query.unwrap_or(&restrictive),
                        update.unwrap_or(default_mutation),
                        delete.unwrap_or(default_mutation),
                    )
                }
                _ => panic!(),
            };

            ResolvedAccess {
                creation: creation.clone(),
                read: read.clone(),
                update: update.clone(),
                delete: delete.clone(),
            }
        }
        None => ResolvedAccess::permissive(),
    }
}