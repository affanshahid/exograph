use id_arena::Id;

use super::column_id::ColumnId;

use super::types::ModelType;

#[derive(Debug, Clone)]
pub enum ModelRelation {
    Pk {
        column_id: ColumnId,
    },
    Scalar {
        column_id: ColumnId,
    },
    ManyToOne {
        column_id: ColumnId,
        other_type_id: Id<ModelType>,
        optional: bool,
    },
    OneToMany {
        column_id: ColumnId,
        other_type_id: Id<ModelType>,
        optional: bool,
    },
}

impl ModelRelation {
    pub fn self_column(&self) -> Option<ColumnId> {
        match self {
            ModelRelation::Pk { column_id } | ModelRelation::Scalar { column_id } => {
                Some(column_id.clone())
            }
            _ => None,
        }
    }
}
