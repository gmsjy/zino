use super::parser;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields};

/// Parses the token stream for the `ModelAccessor` trait derivation.
pub(super) fn parse_token_stream(input: DeriveInput) -> TokenStream {
    // Parsing struct attributes
    let mut composite_constraints = Vec::new();
    for attr in input.attrs.iter() {
        for (key, value) in parser::parse_schema_attr(attr).into_iter() {
            if let Some(value) = value
                && key == "unique_on"
            {
                let mut fields = Vec::new();
                let column_values = value
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .split(',')
                    .map(|s| {
                        let field = s.trim();
                        let field_ident = format_ident!("{}", field);
                        fields.push(field);
                        quote! {
                            (#field, self.#field_ident.to_string().into())
                        }
                    })
                    .collect::<Vec<_>>();
                let composite_field = fields.join("_");
                composite_constraints.push(quote! {
                    let columns = [#(#column_values),*];
                    if !self.is_unique_on(columns).await? {
                        validation.record(#composite_field, "the composite values should be unique");
                    }
                });
            }
        }
    }

    // Parsing field attributes
    let name = input.ident;
    let mut column_methods = Vec::new();
    let mut snapshot_fields = Vec::new();
    let mut snapshot_entries = Vec::new();
    let mut field_constraints = Vec::new();
    let mut populated_queries = Vec::new();
    let mut populated_one_queries = Vec::new();
    let mut primary_key_type = String::from("Uuid");
    let mut primary_key_name = String::from("id");
    let mut user_id_type = String::new();
    if let Data::Struct(data) = input.data
        && let Fields::Named(fields) = data.fields
    {
        let mut model_references: Vec<(String, Vec<String>)> = Vec::new();
        for field in fields.named.into_iter() {
            let type_name = parser::get_type_name(&field.ty);
            if let Some(ident) = field.ident
                && !type_name.is_empty()
            {
                let name = ident.to_string();
                let mut field_alias = None;
                for attr in field.attrs.iter() {
                    let arguments = parser::parse_schema_attr(attr);
                    let is_readable = arguments.iter().all(|arg| arg.0 != "write_only");
                    for (key, value) in arguments.into_iter() {
                        match key.as_str() {
                            "alias" => {
                                field_alias = value;
                            }
                            "primary_key" => {
                                primary_key_name = name.clone();
                            }
                            "snapshot" => {
                                let field = name.clone();
                                let field_ident = format_ident!("{}", field);
                                if type_name == "Uuid" {
                                    snapshot_entries.push(quote! {
                                        snapshot.upsert(#field, self.#field_ident.to_string());
                                    });
                                } else if type_name == "Option<Uuid>" {
                                    snapshot_entries.push(quote! {
                                        let snapshot_value = self.#field_ident
                                            .map(|v| v.to_string());
                                        snapshot.upsert(#field, snapshot_value);
                                    });
                                } else if type_name == "Vec<Uuid>" {
                                    snapshot_entries.push(quote! {
                                        let snapshot_value = self.#field_ident.iter()
                                            .map(|v| v.to_string())
                                            .collect::<Vec<_>>();
                                        snapshot.upsert(#field, snapshot_value);
                                    });
                                } else {
                                    snapshot_entries.push(quote! {
                                        snapshot.upsert(#field, self.#field_ident.clone());
                                    });
                                }
                                snapshot_fields.push(field);
                            }
                            "reference" => {
                                if let Some(value) = value {
                                    let model_ident = format_ident!("{}", value);
                                    if type_name == "Uuid" {
                                        field_constraints.push(quote! {
                                            let values = vec![self.#ident.to_string()];
                                            let data = <#model_ident>::filter(values).await?;
                                            if data.len() != 1 {
                                                validation.record(#name, "it is a nonexistent value");
                                            }
                                        });
                                    } else if type_name == "Option<Uuid>"
                                        || type_name == "Option<String>"
                                    {
                                        field_constraints.push(quote! {
                                            if let Some(value) = self.#ident {
                                                let values = vec![value.to_string()];
                                                let data = <#model_ident>::filter(values).await?;
                                                if data.len() != 1 {
                                                    validation.record(#name, "it is a nonexistent value");
                                                }
                                            }
                                        });
                                    } else if type_name == "Vec<Uuid>" || type_name == "Vec<String>"
                                    {
                                        field_constraints.push(quote! {
                                            let values = self.#ident
                                                .iter()
                                                .map(|v| v.to_string())
                                                .collect::<Vec<_>>();
                                            let length = values.len();
                                            if length > 0 {
                                                let data = <#model_ident>::filter(values).await?;
                                                if data.len() != length {
                                                    validation.record(#name, "there are nonexistent values");
                                                }
                                            }
                                        });
                                    } else if parser::check_vec_type(&type_name) {
                                        field_constraints.push(quote! {
                                            let values = self.#ident.clone();
                                            let length = values.len();
                                            if length > 0 {
                                                let data = <#model_ident>::filter(values).await?;
                                                if data.len() != length {
                                                    validation.record(#name, "there are nonexistent values");
                                                }
                                            }
                                        });
                                    } else if parser::check_option_type(&type_name) {
                                        field_constraints.push(quote! {
                                            if let Some(value) = self.#ident {
                                                let values = vec![value.clone()];
                                                let data = <#model_ident>::filter(values).await?;
                                                if data.len() != 1 {
                                                    validation.record(#name, "it is a nonexistent value");
                                                }
                                            }
                                        });
                                    } else {
                                        field_constraints.push(quote! {
                                            let values = vec![self.#ident.clone()];
                                            let data = <#model_ident>::filter(values).await?;
                                            if data.len() != 1 {
                                                validation.record(#name, "it is a nonexistent value");
                                            }
                                        });
                                    }
                                    match model_references.iter_mut().find(|r| r.0 == value) {
                                        Some(r) => r.1.push(name.clone()),
                                        None => model_references.push((value, vec![name.clone()])),
                                    }
                                }
                            }
                            "unique" => {
                                if type_name == "Uuid" {
                                    field_constraints.push(quote! {
                                        let value = self.#ident;
                                        if !value.is_nil() {
                                            let columns = [(#name, value.to_string().into())];
                                            if !self.is_unique_on(columns).await? {
                                                let message = format!("the value `{value}` is not unique");
                                                validation.record(#name, message);
                                            }
                                        }
                                    });
                                } else if type_name == "String" {
                                    field_constraints.push(quote! {
                                        let value = self.#ident.as_str();
                                        if !value.is_empty() {
                                            let columns = [(#name, value.into())];
                                            if !self.is_unique_on(columns).await? {
                                                let message = format!("the value `{value}` is not unique");
                                                validation.record(#name, message);
                                            }
                                        }
                                    });
                                } else if type_name == "Option<String>" {
                                    field_constraints.push(quote! {
                                        if let Some(value) = self.#ident.as_deref() && !value.is_empty() {
                                            let columns = [(#name, value.into())];
                                            if !self.is_unique_on(columns).await? {
                                                let message = format!("the value `{value}` is not unique");
                                                validation.record(#name, message);
                                            }
                                        }
                                    });
                                } else if type_name == "Option<Uuid>" {
                                    field_constraints.push(quote! {
                                        if let Some(value) = self.#ident && !value.is_nil() {
                                            let columns = [(#name, value.to_string().into())];
                                            if !self.is_unique_on(columns).await? {
                                                let message = format!("the value `{value}` is not unique");
                                                validation.record(#name, message);
                                            }
                                        }
                                    });
                                } else if parser::check_option_type(&type_name) {
                                    field_constraints.push(quote! {
                                        if let Some(value) = self.#ident {
                                            let columns = [(#name, value.into())];
                                            if !self.is_unique_on(columns).await? {
                                                let message = format!("the value `{value}` is not unique");
                                                validation.record(#name, message);
                                            }
                                        }
                                    });
                                } else {
                                    field_constraints.push(quote! {
                                        let value = self.#ident;
                                        let columns = [(#name, value.into())];
                                        if !self.is_unique_on(columns).await? {
                                            let message = format!("the value `{value}` is not unique");
                                            validation.record(#name, message);
                                        }
                                    });
                                }
                            }
                            "not_null" if is_readable => {
                                if type_name == "String" {
                                    field_constraints.push(quote! {
                                        if self.#ident.is_empty() {
                                            validation.record(#name, "it should be nonempty");
                                        }
                                    });
                                } else if type_name == "Uuid" {
                                    field_constraints.push(quote! {
                                        if self.#ident.is_nil() {
                                            validation.record(#name, "it should not be nil");
                                        }
                                    });
                                }
                            }
                            "nonempty" if is_readable => {
                                if parser::check_vec_type(&type_name)
                                    || matches!(type_name.as_str(), "String" | "Map")
                                {
                                    field_constraints.push(quote! {
                                        if self.#ident.is_empty() {
                                            validation.record(#name, "it should be nonempty");
                                        }
                                    });
                                }
                            }
                            "validator" if type_name == "String" => {
                                if let Some(value) = value {
                                    if let Some((validator, validator_fn)) = value.split_once("::")
                                    {
                                        let validator_ident = format_ident!("{}", validator);
                                        let validator_fn_ident = format_ident!("{}", validator_fn);
                                        field_constraints.push(quote! {
                                            if !self.#ident.is_empty() {
                                                let validator = <#validator_ident>::#validator_fn_ident();
                                                if let Err(err) = validator.validate(self.#ident.as_str()) {
                                                    validation.record_fail(#name, err);
                                                }
                                            }
                                        });
                                    } else {
                                        let validator_ident = format_ident!("{}", value);
                                        field_constraints.push(quote! {
                                            if !self.#ident.is_empty() {
                                                if let Err(err) = #validator_ident.validate(self.#ident.as_str()) {
                                                    validation.record_fail(#name, err);
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                            "format" if type_name == "String" => {
                                field_constraints.push(quote! {
                                    if !self.#ident.is_empty() {
                                        validation.validate_format(#name, self.#ident.as_str(), #value);
                                    }
                                });
                            }
                            "length" => {
                                let length = value
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or_default();
                                if type_name == "String" {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if self.#ident.len() != length {
                                            let message = format!("the length should be {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                } else if type_name == "Option<String>" {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if let Some(ref s) = self.#ident && s.len() != length {
                                            let message = format!("the length should be {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                }
                            }
                            "max_length" => {
                                let length = value
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or_default();
                                if type_name == "String" {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if self.#ident.len() > length {
                                            let message = format!("the length should be at most {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                } else if type_name == "Option<String>" {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if let Some(ref s) = self.#ident && s.len() > length {
                                            let message = format!("the length should be at most {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                }
                            }
                            "min_length" => {
                                let length = value
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or_default();
                                if type_name == "String" {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if self.#ident.len() < length {
                                            let message = format!("the length should be at least {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                } else if type_name == "Option<String>" {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if let Some(ref s) = self.#ident && s.len() < length {
                                            let message = format!("the length should be at least {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                }
                            }
                            "max_items" => {
                                if let Some(length) = value.and_then(|s| s.parse::<usize>().ok())
                                    && parser::check_vec_type(&type_name)
                                {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if self.#ident.len() > length {
                                            let message = format!("the length should be at most {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                }
                            }
                            "min_items" => {
                                if let Some(length) = value.and_then(|s| s.parse::<usize>().ok())
                                    && parser::check_vec_type(&type_name)
                                {
                                    field_constraints.push(quote! {
                                        let length = #length;
                                        if self.#ident.len() < length {
                                            let message = format!("the length should be at least {length}");
                                            validation.record(#name, message);
                                        }
                                    });
                                }
                            }
                            "unique_items" => {
                                if parser::check_vec_type(&type_name) {
                                    field_constraints.push(quote! {
                                        let slice = self.#ident.as_slice();
                                        for index in 1..slice.len() {
                                            if slice[index..].contains(&slice[index - 1]) {
                                                let message = format!("array items should be unique");
                                                validation.record(#name, message);
                                                break;
                                            }
                                        }
                                    });
                                }
                            }
                            _ => (),
                        }
                    }
                }
                if primary_key_name == name {
                    primary_key_type = type_name;
                } else {
                    let name_ident = format_ident!("{}", name);
                    let mut snapshot_field = None;
                    match field_alias.as_deref().unwrap_or(name.as_str()) {
                        "name" | "status" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> &str {
                                    self.#name_ident.as_ref()
                                }
                            };
                            column_methods.push(method);
                            snapshot_field = Some(name.as_str());
                        }
                        "namespace" | "visibility" | "description" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> &str {
                                    self.#name_ident.as_ref()
                                }
                            };
                            column_methods.push(method);
                        }
                        "content" | "extra" if type_name == "Map" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> Option<&Map> {
                                    let map = &self.#name_ident;
                                    (!map.is_empty()).then_some(map)
                                }
                            };
                            column_methods.push(method);
                        }
                        "owner_id" | "maintainer_id" => {
                            let user_type_opt = type_name.strip_prefix("Option");
                            let user_type = if let Some(user_type) = user_type_opt {
                                user_type.trim_matches(|c| c == '<' || c == '>').to_owned()
                            } else {
                                type_name.clone()
                            };
                            let user_type_ident = format_ident!("{}", user_type);
                            let method = if user_type_opt.is_some() {
                                quote! {
                                    #[inline]
                                    fn #name_ident(&self) -> Option<&#user_type_ident> {
                                        self.#name_ident.as_ref()
                                    }
                                }
                            } else {
                                quote! {
                                    #[inline]
                                    fn #name_ident(&self) -> Option<&#user_type_ident> {
                                        let id = &self.#name_ident;
                                        (id != &#user_type_ident::default()).then_some(id)
                                    }
                                }
                            };
                            column_methods.push(method);
                            user_id_type = user_type;
                        }
                        "created_at" if type_name == "DateTime" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> DateTime {
                                    self.#name_ident
                                }
                            };
                            column_methods.push(method);
                        }
                        "updated_at" if type_name == "DateTime" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> DateTime {
                                    self.#name_ident
                                }
                            };
                            column_methods.push(method);
                            snapshot_field = Some("updated_at");
                        }
                        "version" if type_name == "u64" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> u64 {
                                    self.#name_ident
                                }
                            };
                            column_methods.push(method);
                            snapshot_field = Some("version");
                        }
                        "edition" if type_name == "u32" => {
                            let method = quote! {
                                #[inline]
                                fn #name_ident(&self) -> u32 {
                                    self.#name_ident
                                }
                            };
                            column_methods.push(method);
                        }
                        _ => (),
                    }
                    if let Some(field) = snapshot_field {
                        let field_ident = format_ident!("{}", field);
                        snapshot_entries.push(quote! {
                            snapshot.upsert(#field, self.#field_ident.clone());
                        });
                        snapshot_fields.push(field.to_owned());
                    }
                }
            }
        }
        if model_references.is_empty() {
            populated_queries.push(quote! {
                let mut models = Self::find::<Map>(query).await?;
                for model in models.iter_mut() {
                    Self::after_decode(model).await?;
                    translate_enabled.then(|| Self::translate_model(model));
                }
            });
            populated_one_queries.push(quote! {
                let mut model = Self::find_by_id::<Map>(id)
                    .await?
                    .ok_or_else(|| zino_core::warn!("404 Not Found: cannot find the model `{}`", id))?;
                Self::after_decode(&mut model).await?;
                Self::translate_model(&mut model);
            });
        } else {
            populated_queries.push(quote! {
                let mut models = Self::find::<Map>(query).await?;
                for model in models.iter_mut() {
                    Self::after_decode(model).await?;
                    translate_enabled.then(|| Self::translate_model(model));
                }
            });
            populated_one_queries.push(quote! {
                let mut model = Self::find_by_id::<Map>(id)
                    .await?
                    .ok_or_else(|| zino_core::warn!("404 Not Found: cannot find the model `{}`", id))?;
                Self::after_decode(&mut model).await?;
                Self::translate_model(&mut model);
            });
            for (model, ref_fields) in model_references.into_iter() {
                let model_ident = format_ident!("{}", model);
                let populated_query = quote! {
                    let mut query = #model_ident::default_snapshot_query();
                    query.add_filter("translate", translate_enabled);
                    #model_ident::populate(&mut query, &mut models, [#(#ref_fields),*]).await?;
                };
                let populated_one_query = quote! {
                    let mut query = #model_ident::default_query();
                    query.add_filter("translate", true);
                    #model_ident::populate_one(&mut query, &mut model, [#(#ref_fields),*]).await?;
                };
                populated_queries.push(populated_query);
                populated_one_queries.push(populated_one_query);
            }
        }
        populated_queries.push(quote! { Ok(models) });
        populated_one_queries.push(quote! { Ok(model) });
    }
    if user_id_type.is_empty() {
        user_id_type = primary_key_type.clone();
    }

    // Output
    let model_primary_key_type = format_ident!("{}", primary_key_type);
    let model_primary_key = format_ident!("{}", primary_key_name);
    let model_user_id_type = format_ident!("{}", user_id_type);
    quote! {
        use zino_core::{
            model::Query,
            orm::{ModelAccessor, ModelHelper as _},
            validation::Validation as ZinoValidation,
            Map as ZinoMap,
        };

        impl ModelAccessor<#model_primary_key_type, #model_user_id_type> for #name {
            #[inline]
            fn id(&self) -> &#model_primary_key_type {
                &self.#model_primary_key
            }

            #(#column_methods)*

            fn snapshot(&self) -> Map {
                let mut snapshot = Map::new();
                snapshot.upsert(Self::PRIMARY_KEY_NAME, self.primary_key_value());
                #(#snapshot_entries)*
                snapshot
            }

            fn default_snapshot_query() -> Query {
                let mut query = Self::default_query();
                let fields = [
                    Self::PRIMARY_KEY_NAME,
                    #(#snapshot_fields),*
                ];
                query.allow_fields(&fields);
                query
            }

            async fn check_constraints(&self) -> Result<ZinoValidation, ZinoError> {
                let mut validation = ZinoValidation::new();
                if self.id() == &<#model_primary_key_type>::default()
                    && !Self::primary_key_column().auto_increment()
                {
                    validation.record(Self::PRIMARY_KEY_NAME, "should not be a default value");
                }
                #(#composite_constraints)*
                #(#field_constraints)*
                Ok(validation)
            }

            async fn fetch(query: &Query) -> Result<Vec<ZinoMap>, ZinoError> {
                let translate_enabled = query.translate_enabled();
                #(#populated_queries)*
            }

            async fn fetch_by_id(id: &#model_primary_key_type) -> Result<ZinoMap, ZinoError> {
                #(#populated_one_queries)*
            }
        }
    }
}
