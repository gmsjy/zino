use super::Tag;
use serde::{Deserialize, Serialize};
use zino::prelude::*;
use zino_derive::{Model, ModelAccessor, ModelHooks, Schema};
use zino_model::user::JwtAuthService;

/// The `User` model.
#[derive(
    Debug, Clone, Default, Serialize, Deserialize, Schema, ModelAccessor, ModelHooks, Model,
)]
#[serde(default)]
pub struct User {
    // Basic fields.
    #[schema(primary_key, auto_increment, readonly)]
    id: i64,
    #[schema(not_null, index_type = "text", comment = "User name")]
    name: String,
    #[schema(default_value = "Inactive", index_type = "hash")]
    status: String,
    #[schema(index_type = "text")]
    description: String,

    // Info fields.
    #[schema(not_null, unique, writeonly, constructor = "AccessKeyId::new")]
    access_key_id: String,
    #[schema(not_null, unique, writeonly, comment = "User account")]
    account: String,
    #[schema(not_null, writeonly, comment = "User password")]
    password: String,
    mobile: String,
    email: String,
    avatar: String,
    #[schema(snapshot, nonempty, comment = "User roles")]
    roles: Vec<String>,
    #[schema(reference = "Tag", comment = "User tags")]
    tags: Vec<i64>,

    // Security.
    #[schema(generated)]
    last_login_at: DateTime,
    #[schema(generated)]
    last_login_ip: String,
    #[schema(generated)]
    current_login_at: DateTime,
    #[schema(generated)]
    current_login_ip: String,
    #[schema(generated)]
    login_count: u32,

    // Extensions.
    #[schema(reserved)]
    content: Map,
    #[schema(reserved)]
    extra: Map,

    // Revisions.
    #[schema(readonly, default_value = "now", index_type = "btree")]
    created_at: DateTime,
    #[schema(default_value = "now", index_type = "btree")]
    updated_at: DateTime,
    version: u64,
}

impl JwtAuthService<i64> for User {
    const LOGIN_AT_FIELD: Option<&'static str> = Some("current_login_at");
    const LOGIN_IP_FIELD: Option<&'static str> = Some("current_login_ip");
}
