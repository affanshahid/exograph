//! Build mutation input types (<Type>CreationInput, <Type>UpdateInput, <Type>ReferenceInput) and
//! mutations (create<Type>, update<Type>, and delete<Type> as well as their plural versions)

use super::naming::ToPostgresTypeNames;
use super::resolved_builder::ResolvedFieldType;
use super::type_builder::ResolvedTypeEnv;
use core_model::mapped_arena::{MappedArena, SerializableSlabIndex};
use postgres_model::operation::{OperationReturnType, PostgresMutation, PostgresMutationKind};
use postgres_model::relation::PostgresRelation;
use postgres_model::types::{
    PostgresCompositeType, PostgresField, PostgresFieldType, PostgresType, PostgresTypeKind,
    PostgresTypeModifier,
};

use super::builder::Builder;
use super::create_mutation_builder::CreateMutationBuilder;
use super::delete_mutation_builder::DeleteMutationBuilder;
use super::reference_input_type_builder::ReferenceInputTypeBuilder;
use super::resolved_builder::{ResolvedCompositeType, ResolvedType};
use super::system_builder::SystemContextBuilding;
use super::update_mutation_builder::UpdateMutationBuilder;

// TODO: Introduce this module as a struct (and have it hold the sub-builders)
// TODO: Abstract the concept of composite builders

/// Build shallow mutation input types
pub fn build_shallow(models: &MappedArena<ResolvedType>, building: &mut SystemContextBuilding) {
    ReferenceInputTypeBuilder {}.build_shallow_only_persistent(models, building);

    CreateMutationBuilder {}.build_shallow_only_persistent(models, building);
    UpdateMutationBuilder {}.build_shallow_only_persistent(models, building);
    DeleteMutationBuilder {}.build_shallow_only_persistent(models, building);
}

/// Expand the mutation input types as well as build the mutation
pub fn build_expanded(resolved_env: &ResolvedTypeEnv, building: &mut SystemContextBuilding) {
    ReferenceInputTypeBuilder {}.build_expanded(resolved_env, building); // Used by many...

    CreateMutationBuilder {}.build_expanded(resolved_env, building);
    UpdateMutationBuilder {}.build_expanded(resolved_env, building);
    DeleteMutationBuilder {}.build_expanded(resolved_env, building);
}

pub trait MutationBuilder {
    fn single_mutation_name(model_type: &PostgresType) -> String;
    fn single_mutation_kind(
        model_type_id: SerializableSlabIndex<PostgresType>,
        model_type: &PostgresType,
        building: &SystemContextBuilding,
    ) -> PostgresMutationKind;

    fn multi_mutation_name(model_type: &PostgresType) -> String;
    fn multi_mutation_kind(
        model_type_id: SerializableSlabIndex<PostgresType>,
        model_type: &PostgresType,
        building: &SystemContextBuilding,
    ) -> PostgresMutationKind;

    fn build_mutations(
        &self,
        model_type_id: SerializableSlabIndex<PostgresType>,
        model_type: &PostgresType,
        building: &SystemContextBuilding,
    ) -> Vec<PostgresMutation> {
        let single_mutation = PostgresMutation {
            name: Self::single_mutation_name(model_type),
            kind: Self::single_mutation_kind(model_type_id, model_type, building),
            return_type: OperationReturnType {
                type_id: model_type_id,
                type_name: model_type.name.clone(),
                is_primitive: false,
                type_modifier: PostgresTypeModifier::Optional,
            },
        };

        let multi_mutation = PostgresMutation {
            name: Self::multi_mutation_name(model_type),
            kind: Self::multi_mutation_kind(model_type_id, model_type, building),
            return_type: OperationReturnType {
                type_id: model_type_id,
                type_name: model_type.name.clone(),
                is_primitive: false,
                type_modifier: PostgresTypeModifier::List,
            },
        };

        vec![single_mutation, multi_mutation]
    }
}

pub trait DataParamBuilder<D> {
    fn data_param_type_name(resolved_composite_type: &ResolvedCompositeType) -> String {
        Self::base_data_type_name(&resolved_composite_type.name)
    }

    fn base_data_type_name(model_type_name: &str) -> String;

    fn data_param(model_type: &PostgresType, building: &SystemContextBuilding, array: bool) -> D;

    fn data_type_name(model_type_name: &str, container_type: Option<&str>) -> String {
        let base_name = Self::base_data_type_name(model_type_name);
        super::mutation_builder::data_type_name(&base_name, container_type)
    }

    fn compute_data_fields(
        &self,
        postgres_fields: &[PostgresField],
        top_level_type: Option<&PostgresType>,
        container_type: Option<&str>,
        building: &SystemContextBuilding,
    ) -> Vec<PostgresField> {
        postgres_fields
            .iter()
            .flat_map(|field| {
                self.compute_data_field(field, top_level_type, container_type, building)
            })
            .collect()
    }

    // TODO: Revisit this after nested update mutation works
    fn mark_fields_optional() -> bool;

    fn data_param_field_type_names(
        &self,
        resolved_composite_type: &ResolvedCompositeType,
        resolved_types: &MappedArena<ResolvedType>,
    ) -> Vec<String> {
        resolved_composite_type
            .fields
            .iter()
            .flat_map(|field| {
                // Create a nested input data type only if it refers to a many side
                // So for Venue <-> [Concert] case, create only ConcertCreationInputFromVenue

                // we can treat Optional fields as their inner type for the purposes of
                // computing their type names
                let typ = match &field.typ {
                    ResolvedFieldType::Optional(inner_type) => inner_type.as_ref(),
                    _ => &field.typ,
                };

                // If the type is a list or a reference, we need to create a nested input type (one-to-many or one-to-zero-or-one)
                if let ResolvedFieldType::List(_) | ResolvedFieldType::Optional(_) = field.typ {
                    if let Some(ResolvedType::Composite(ResolvedCompositeType { name, .. })) =
                        typ.deref_subsystem_type(resolved_types)
                    {
                        Self::data_param_field_one_to_many_type_names(name, resolved_composite_type)
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            })
            .collect()
    }

    fn data_param_field_one_to_many_type_names(
        field_type_name: &str,
        resolved_composite_type: &ResolvedCompositeType,
    ) -> Vec<String> {
        vec![Self::data_type_name(
            field_type_name,
            Some(&resolved_composite_type.name),
        )]
    }

    fn compute_data_field(
        &self,
        field: &PostgresField,
        top_level_type: Option<&PostgresType>,
        container_type: Option<&str>,
        building: &SystemContextBuilding,
    ) -> Option<PostgresField> {
        let optional = Self::mark_fields_optional() || field.has_default_value;

        match &field.relation {
            PostgresRelation::Pk { .. } => None, // TODO: Make this decision based on autoincrement/uuid etc of the id
            PostgresRelation::Scalar { .. } => Some(PostgresField {
                typ: if optional {
                    field.typ.optional()
                } else {
                    field.typ.clone()
                },
                ..field.clone()
            }),
            PostgresRelation::OneToMany { .. } => {
                self.compute_one_to_many_data_field(field, container_type, building)
            }
            PostgresRelation::ManyToOne { .. } => {
                let field_type_name = field.typ.type_name().reference_type();
                let field_type_id = building.mutation_types.get_id(&field_type_name).unwrap();
                let field_plain_type = PostgresFieldType::Reference {
                    type_name: field_type_name,
                    is_primitive: false,
                    type_id: field_type_id,
                };
                let field_type = match field.typ {
                    PostgresFieldType::Reference { .. } => {
                        if optional {
                            field_plain_type.optional()
                        } else {
                            field_plain_type
                        }
                    }
                    PostgresFieldType::Optional(_) => {
                        PostgresFieldType::Optional(Box::new(field_plain_type))
                    }
                    PostgresFieldType::List(_) => {
                        PostgresFieldType::List(Box::new(field_plain_type))
                    }
                };

                match &top_level_type {
                    Some(value) if value.name == field.typ.type_name() => None,
                    _ => Some(PostgresField {
                        name: field.name.clone(),
                        typ: field_type,
                        relation: field.relation.clone(),
                        has_default_value: field.has_default_value,
                    }),
                }
            }
        }
    }

    fn compute_one_to_many_data_field(
        &self,
        field: &PostgresField,
        container_type: Option<&str>,
        building: &SystemContextBuilding,
    ) -> Option<PostgresField> {
        let optional =
            matches!(field.typ, PostgresFieldType::Optional(_)) || Self::mark_fields_optional();

        let field_type_name = Self::data_type_name(field.typ.type_name(), container_type);

        building
            .mutation_types
            .get_id(&field_type_name)
            .and_then(|field_type_id| {
                let field_plain_type = PostgresFieldType::Reference {
                    type_name: field_type_name,
                    is_primitive: false, // Mutation types are never primitive
                    type_id: field_type_id,
                };
                let field_type = PostgresFieldType::List(Box::new(field_plain_type));

                match &container_type {
                    Some(value) if value == &field.typ.type_name() => None,
                    _ => Some(PostgresField {
                        name: field.name.clone(),
                        typ: if optional {
                            field_type.optional()
                        } else {
                            field_type
                        },
                        relation: field.relation.clone(),
                        has_default_value: field.has_default_value,
                    }),
                }
            })
    }

    fn expanded_data_type(
        &self,
        model_type: &PostgresType,
        resolved_env: &ResolvedTypeEnv,
        building: &SystemContextBuilding,
        top_level_type: Option<&PostgresType>,
        container_type: Option<&PostgresType>,
    ) -> Vec<(SerializableSlabIndex<PostgresType>, PostgresCompositeType)> {
        if let PostgresTypeKind::Composite(PostgresCompositeType {
            fields,
            table_id,
            pk_query,
            collection_query,
            access,
        }) = &model_type.kind
        {
            let model_fields = fields;

            let mut field_types: Vec<_> = model_fields
                .iter()
                .flat_map(|field| {
                    let field_type = field.typ.base_type(&building.postgres_types.values);
                    if let (PostgresTypeKind::Composite(_), PostgresRelation::OneToMany { .. }) =
                        (&field_type.kind, &field.relation)
                    {
                        self.expand_one_to_many(
                            model_type,
                            field,
                            field_type,
                            resolved_env,
                            building,
                            top_level_type,
                            Some(model_type),
                        )
                    } else {
                        vec![]
                    }
                })
                .collect();

            let existing_type_name = Self::data_type_name(
                model_type.name.as_str(),
                container_type.map(|value| value.name.as_str()),
            );
            let existing_type_id = building.mutation_types.get_id(&existing_type_name).unwrap();

            let input_type_fields = self.compute_data_fields(
                model_fields,
                top_level_type,
                Some(model_type.name.as_str()),
                building,
            );
            field_types.push((
                existing_type_id,
                PostgresCompositeType {
                    fields: input_type_fields,
                    table_id: *table_id,
                    pk_query: *pk_query,
                    collection_query: *collection_query,
                    access: access.clone(),
                },
            ));

            field_types
        } else {
            vec![]
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_one_to_many(
        &self,
        model_type: &PostgresType,
        _field: &PostgresField,
        field_type: &PostgresType,
        resolved_env: &ResolvedTypeEnv,
        building: &SystemContextBuilding,
        top_level_type: Option<&PostgresType>,
        _container_type: Option<&PostgresType>,
    ) -> Vec<(SerializableSlabIndex<PostgresType>, PostgresCompositeType)> {
        let new_container_type = Some(model_type);

        let existing_type_name = Self::data_type_name(
            &field_type.name,
            new_container_type.map(|value| value.name.as_str()),
        );

        if let PostgresTypeKind::Primitive = building
            .mutation_types
            .get_by_key(&existing_type_name)
            .unwrap_or_else(|| panic!("Could not find type {} to expand", existing_type_name))
            .kind
        {
            // If not already expanded (i.e. the kind is primitive)
            self.expanded_data_type(
                field_type,
                resolved_env,
                building,
                top_level_type,
                new_container_type,
            )
        } else {
            vec![]
        }
    }
}

pub fn create_data_type_name(model_type_name: &str, container_type: Option<&str>) -> String {
    let base_name = model_type_name.creation_type();
    data_type_name(&base_name, container_type)
}

pub fn update_data_type_name(model_type_name: &str, container_type: Option<&str>) -> String {
    let base_name = model_type_name.update_type();
    data_type_name(&base_name, container_type)
}

fn data_type_name(base_name: &str, container_type: Option<&str>) -> String {
    match container_type {
        Some(container_type) => {
            format!("{}From{}", base_name, container_type)
        }
        None => base_name.to_owned(),
    }
}