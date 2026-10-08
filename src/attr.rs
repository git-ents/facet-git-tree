//! The crate's authoring surface: the attributes a schema author declares on a
//! Rust type. `facet` permits one attribute grammar per crate, so all are
//! declared here and written `#[facet(facet_git_tree::…)]`.
//!
//! Defaults are not expressible here (`facet`'s grammar accepts only strings
//! and bools) and are supplied through
//! [`Hints::defaulted`](crate::migration::Hints::defaulted) instead.

facet::define_attr_grammar! {
    ns "git_tree";
    crate_path ::facet_git_tree::attr;

    pub enum Attr {
        /// The source-side field name a struct field was renamed from:
        /// `#[facet(facet_git_tree::renamed_from = "old_name")]`.
        RenamedFrom(&'static str),

        /// Marks the annotated field, or the annotated type, as identity- or
        /// key-bearing: `#[facet(facet_git_tree::identity_key)]`.
        IdentityKey,
    }
}

/// The namespace every attribute of this grammar is stored under.
const NS: Option<&str> = Some("git_tree");

/// The rename hint `field` declares via
/// `#[facet(facet_git_tree::renamed_from = …)]`, if any.
pub(crate) fn renamed_from(field: &'static facet::Field) -> Option<&'static str> {
    field
        .attributes
        .iter()
        .find(|attr| attr.ns() == NS && attr.key() == "renamed_from")
        .and_then(|attr| attr.get_as::<&'static str>())
        .copied()
}

/// Whether `attributes` carries `#[facet(facet_git_tree::identity_key)]`.
pub(crate) fn is_identity_key(attributes: &'static [facet::Attr]) -> bool {
    attributes
        .iter()
        .any(|attr| attr.ns() == NS && attr.key() == "identity_key")
}
