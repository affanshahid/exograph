//#[cfg(test)] // needed until we use this code from `main`
pub mod common_test_data {
    use crate::model::types::ModelRelation::*;
    use crate::model::types::ModelTypeModifier::*;
    use crate::model::types::{ModelField, ModelType};
    use crate::model::{system::ModelSystem, types::ModelTypeKind::*};

    pub fn test_system() -> ModelSystem {
        let mut system = ModelSystem::new();

        system.add_type(create_venue_model_type(&system));
        system.add_type(create_concert_model_type(&system));

        system.build();

        system
    }

    fn create_concert_model_type(system: &ModelSystem) -> ModelType {
        ModelType {
            name: "Concert".to_string(),
            kind: Composite {
                model_fields: vec![
                    ModelField {
                        name: "id".to_string(),
                        type_name: system.int_type(),
                        type_modifier: NonNull,
                        relation: Pk { column_name: None },
                    },
                    ModelField {
                        name: "title".to_string(),
                        type_name: system.string_type(),
                        type_modifier: NonNull,
                        relation: Scalar { column_name: None },
                    },
                    ModelField {
                        name: "venue".to_string(),
                        type_name: "Venue".to_string(),
                        type_modifier: NonNull,
                        relation: ManyToOne {
                            column_name: Some("venueid".to_string()),
                        },
                    },
                ],
            },
        }
    }

    fn create_venue_model_type(system: &ModelSystem) -> ModelType {
        ModelType {
            name: "Venue".to_string(),
            kind: Composite {
                model_fields: vec![
                    ModelField {
                        name: "id".to_string(),
                        type_name: system.int_type(),
                        type_modifier: NonNull,
                        relation: Pk { column_name: None },
                    },
                    ModelField {
                        name: "name".to_string(),
                        type_name: system.string_type(),
                        type_modifier: Optional,
                        relation: Scalar { column_name: None },
                    },
                ],
            },
        }
    }
}
