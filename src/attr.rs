//! The crate's authoring surface: the attributes a schema author declares on a
//! Rust type. `facet` permits one attribute grammar per crate, so all are
//! declared here and written `#[facet(facet_git_tree::…)]`.

facet::define_attr_grammar! {
    ns "git_tree";
    crate_path ::facet_git_tree::attr;

    pub enum Attr {
        /// Marks the annotated field, or the annotated type, as identity- or
        /// key-bearing: `#[facet(facet_git_tree::identity_key)]`.
        IdentityKey,
    }
}

/// The namespace every attribute of this grammar is stored under.
const NS: Option<&str> = Some("git_tree");

/// Whether `attributes` carries `#[facet(facet_git_tree::identity_key)]`.
pub(crate) fn is_identity_key(attributes: &'static [facet::Attr]) -> bool {
    attributes
        .iter()
        .any(|attr| attr.ns() == NS && attr.key() == "identity_key")
}
