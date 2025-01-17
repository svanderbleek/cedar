/*
 * Copyright 2022-2023 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::fmt::Display;

use super::SchemaType;
use crate::ast::{EntityUID, Expr, ExprKind, Name, PolicyID, RestrictedExpr, RestrictedExprError};
use crate::entities::conformance::{EntitySchemaConformanceError, HeterogeneousSetError};
use crate::extensions::ExtensionFunctionLookupError;
use crate::parser::err::ParseErrors;
use either::Either;
use itertools::Itertools;
use smol_str::SmolStr;
use thiserror::Error;

/// Escape kind
#[derive(Debug)]
pub enum EscapeKind {
    /// Escape `__entity`
    Entity,
    /// Escape `__extn`
    Extension,
}

impl Display for EscapeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Entity => write!(f, "__entity"),
            Self::Extension => write!(f, "__extn"),
        }
    }
}

/// Errors thrown during deserialization from JSON
#[derive(Debug, Error)]
pub enum JsonDeserializationError {
    /// Error thrown by the `serde_json` crate
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    /// Contents of an escape failed to parse.
    #[error("failed to parse escape `{kind}`: {value}, errors: {errs}")]
    ParseEscape {
        /// Escape kind
        kind: EscapeKind,
        /// Escape value at fault
        value: String,
        /// Parse errors
        errs: ParseErrors,
    },
    /// Restricted expression error
    #[error(transparent)]
    RestrictedExpressionError(#[from] RestrictedExprError),
    /// A field that needs to be a literal entity reference, was some other JSON value
    #[error("{ctx}, expected a literal entity reference, but got `{}`", display_json_value(.got.as_ref()))]
    ExpectedLiteralEntityRef {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// the expression we got instead
        got: Box<Either<serde_json::Value, Expr>>,
    },
    /// A field that needs to be an extension value, was some other JSON value
    #[error("{ctx}, expected an extension value, but got `{}`", display_json_value(.got.as_ref()))]
    ExpectedExtnValue {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// the expression we got instead
        got: Box<Either<serde_json::Value, Expr>>,
    },
    /// Contexts need to be records, but we got some other JSON value
    #[error("expected `context` to be a record, but got `{got}`")]
    ExpectedContextToBeRecord {
        /// Expression we got instead
        got: Box<RestrictedExpr>,
    },
    /// Parents of actions should be actions, but this action has a non-action parent
    #[error("action `{uid}` has a non-action parent `{parent}`")]
    ActionParentIsNotAction {
        /// Action entity that had the invalid parent
        uid: EntityUID,
        /// Parent that is invalid
        parent: EntityUID,
    },
    /// Schema-based parsing needed an implicit extension constructor, but no suitable
    /// constructor was found
    #[error("{ctx}, missing extension constructor for {arg_type} -> {return_type}")]
    MissingImpliedConstructor {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// return type of the constructor we were looking for
        return_type: Box<SchemaType>,
        /// argument type of the constructor we were looking for
        arg_type: Box<SchemaType>,
    },
    /// The same key appears two or more times in a single record literal
    #[error("{ctx}, duplicate key `{key}` in record literal")]
    DuplicateKeyInRecordLiteral {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// The key that appeared two or more times
        key: SmolStr,
    },
    /// During schema-based parsing, encountered an entity which does not
    /// conform to the schema.
    ///
    /// This error contains the `Entity` analogues some of the other errors
    /// listed below, among other things.
    #[error(transparent)]
    EntitySchemaConformance(EntitySchemaConformanceError),
    /// During schema-based parsing, encountered this attribute on a record, but
    /// that attribute shouldn't exist on that record
    #[error("{ctx}, record attribute `{record_attr}` should not exist according to the schema")]
    UnexpectedRecordAttr {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// Name of the (Record) attribute which was unexpected
        record_attr: SmolStr,
    },
    /// During schema-based parsing, didn't encounter this attribute of a
    /// record, but that attribute should have existed
    #[error("{ctx}, expected the record to have an attribute `{record_attr}`, but it does not")]
    MissingRequiredRecordAttr {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// Name of the (Record) attribute which was expected
        record_attr: SmolStr,
    },
    /// During schema-based parsing, found a different type than the schema indicated.
    ///
    /// (This is used in all cases except inside entity attributes; type mismatches in
    /// entity attributes are reported as `Self::EntitySchemaConformance`. As of
    /// this writing, that means this should only be used for schema-based
    /// parsing of the `Context`.)
    #[error("{ctx}, type mismatch: expected type {expected}, but actually has type {actual}")]
    TypeMismatch {
        /// Context of this error, which will be something other than `EntityAttribute`.
        /// (Type mismatches in entity attributes are reported as
        /// `Self::EntitySchemaConformance`.)
        ctx: Box<JsonDeserializationErrorContext>,
        /// Type which was expected
        expected: Box<SchemaType>,
        /// Type which was encountered instead
        actual: Box<SchemaType>,
    },
    /// During schema-based parsing, found a set whose elements don't all have
    /// the same type.  This doesn't match any possible schema.
    ///
    /// (This is used in all cases except inside entity attributes;
    /// heterogeneous sets in entity attributes are reported as
    /// `Self::EntitySchemaConformance`. As of this writing, that means this
    /// should only be used for schema-based parsing of the `Context`. Note that
    /// for non-schema-based parsing, heterogeneous sets are not an error.)
    #[error("{ctx}, {err}")]
    HeterogeneousSet {
        /// Context of this error, which will be something other than `EntityAttribute`.
        /// (Heterogeneous sets in entity attributes are reported as
        /// `Self::EntitySchemaConformance`.)
        ctx: Box<JsonDeserializationErrorContext>,
        /// Underlying error
        err: HeterogeneousSetError,
    },
    /// During schema-based parsing, error looking up an extension function.
    /// This error can occur during schema-based parsing because that may
    /// require getting information about any extension functions referenced in
    /// the JSON.
    ///
    /// (This is used in all cases except inside entity attributes; extension
    /// function lookup errors in entity attributes are reported as
    /// `Self::EntitySchemaConformance`. As of this writing, that means this
    /// should only be used for schema-based parsing of the `Context`.)
    #[error("{ctx}, {err}")]
    ExtensionFunctionLookup {
        /// Context of this error, which will be something other than
        /// `EntityAttribute`.
        /// (Extension function lookup errors in entity attributes are reported
        /// as `Self::EntitySchemaConformance`.)
        ctx: Box<JsonDeserializationErrorContext>,
        /// Underlying error
        err: ExtensionFunctionLookupError,
    },
    /// Raised when a JsonValue contains the no longer supported `__expr` escape
    #[error("{0}, invalid escape. The `__expr` escape is no longer supported")]
    ExprTag(Box<JsonDeserializationErrorContext>),
}

/// Errors thrown during serialization to JSON
#[derive(Debug, Error)]
pub enum JsonSerializationError {
    /// Error thrown by `serde_json`
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    /// Extension-function calls with 0 arguments are not currently supported in
    /// our JSON format.
    #[error("unsupported call to `{func}`. Extension function calls with 0 arguments are not currently supported in our JSON format")]
    ExtnCall0Arguments {
        /// Name of the function which was called with 0 arguments
        func: Name,
    },
    /// Extension-function calls with 2 or more arguments are not currently
    /// supported in our JSON format.
    #[error("unsupported call to `{func}`. Extension function calls with 2 or more arguments are not currently supported in our JSON format")]
    ExtnCall2OrMoreArguments {
        /// Name of the function which was called with 2 or more arguments
        func: Name,
    },
    /// Encountered a `Record` which can't be serialized to JSON because it
    /// contains a key which is reserved as a JSON escape.
    #[error("record uses reserved key `{key}`")]
    ReservedKey {
        /// Reserved key which was used by the `Record`
        key: SmolStr,
    },
    /// Encountered an `ExprKind` which we didn't expect. Either a case is
    /// missing in `CedarValueJson::from_expr()`, or an internal invariant was
    /// violated and there is a non-restricted expression in `RestrictedExpr`
    #[error("unexpected restricted expression `{kind:?}`")]
    UnexpectedRestrictedExprKind {
        /// `ExprKind` which we didn't expect to find
        kind: ExprKind,
    },
}

/// Gives information about the context of a JSON deserialization error (e.g.,
/// where we were in the JSON document).
#[derive(Debug, Clone)]
pub enum JsonDeserializationErrorContext {
    /// The error occurred while deserializing the attribute `attr` of an entity.
    EntityAttribute {
        /// Entity where the error occurred
        uid: EntityUID,
        /// Attribute where the error occurred
        attr: SmolStr,
    },
    /// The error occurred while deserializing the `parents` field of an entity.
    EntityParents {
        /// Entity where the error occurred
        uid: EntityUID,
    },
    /// The error occurred while deserializing the `uid` field of an entity.
    EntityUid,
    /// The error occurred while deserializing the `Context`.
    Context,
    /// The error occurred while deserializing a policy in JSON (EST) form.
    Policy {
        /// ID of the policy we were deserializing
        id: PolicyID,
    },
}

impl std::fmt::Display for JsonDeserializationErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EntityAttribute { uid, attr } => write!(f, "in attribute `{attr}` on `{uid}`"),
            Self::EntityParents { uid } => write!(f, "in parents field of `{uid}`"),
            Self::EntityUid => write!(f, "in uid field of <unknown entity>"),
            Self::Context => write!(f, "while parsing context"),
            Self::Policy { id } => write!(f, "while parsing JSON policy `{id}`"),
        }
    }
}

fn display_json_value(v: &Either<serde_json::Value, Expr>) -> String {
    match v {
        Either::Left(json) => display_value(json),
        Either::Right(e) => e.to_string(),
    }
}

fn display_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Array(contents) => {
            format!("[{}]", contents.iter().map(display_value).join(", "))
        }
        serde_json::Value::Object(map) => {
            let mut v: Vec<_> = map.iter().collect();
            // We sort the keys here so that our error messages are consistent and defined
            v.sort_by_key(|p| p.0);
            let display_kv = |kv: &(&String, &serde_json::Value)| format!("\"{}\":{}", kv.0, kv.1);
            format!("{{{}}}", v.iter().map(display_kv).join(","))
        }
        other => other.to_string(),
    }
}
