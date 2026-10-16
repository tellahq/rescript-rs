//! `#[derive(Spice)]`: a ReScript declaration with ppx-spice annotations,
//! and a Rust codec that reads and writes JSON as ppx-spice's generated
//! code does for that declaration (see `rescript_rs::spice`).

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    ext::IdentExt, spanned::Spanned, Attribute, Data, DeriveInput, Error, Expr, Fields,
    GenericArgument, Ident, LitStr, Path, PathArguments, Result, Type,
};

const RESCRIPT_KEYWORDS: &[&str] = &[
    "and",
    "as",
    "assert",
    "await",
    "constraint",
    "else",
    "exception",
    "external",
    "false",
    "for",
    "if",
    "in",
    "include",
    "lazy",
    "let",
    "module",
    "mutable",
    "of",
    "open",
    "private",
    "rec",
    "switch",
    "true",
    "try",
    "type",
    "when",
    "while",
    "with",
];

/// Keys a record field cannot have: spice reads `dict[key]`, which finds
/// these on `Object.prototype` when the JSON lacks them.
const PROTOTYPE_KEYS: &[&str] = &[
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "__proto__",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];

#[derive(Default)]
struct ContainerAttr {
    module: Option<String>,
    name: Option<String>,
    attrs: Option<String>,
    /// `gen_type`: also export the type to TypeScript, `@genType`.
    gen_type: bool,
    /// How field names become JSON keys (and ReScript field names).
    rename_fields: Option<RenameRule>,
    /// How variant names become their JSON names.
    rename_variants: Option<RenameRule>,
    /// `@spice.decode` or `@spice.encode` instead of `@spice`: ReScript gets
    /// only that function (Rust gets both).
    only: Option<&'static str>,
    /// `@spice.serde({legacy: true})`: an enum's decoders also read the
    /// default spice encoding, `["Constructor", …]`.
    legacy: bool,
    /// `@spice.serde({legacyWrite: true})`: `legacy`, and the encoders write
    /// the default spice encoding.
    legacy_write: bool,
    /// `@tag(…)`: internally tagged.
    tag: Option<String>,
    krate: Option<Path>,
    /// A polymorphic variant, `[#A | #B]`.
    poly: bool,
    /// `runtime_names`: a renamed constructor is also called by its JSON
    /// name at runtime, ReScript's `@as(…)`; otherwise only its JSON name
    /// changes, `@spice.as(…)`.
    runtime_names: bool,
    /// `#[rescript(fallback = "path")]`: what anything the decoder rejects
    /// decodes to, `@spice.fallback(…)`.
    fallback: Option<Path>,
}

#[derive(Default)]
struct FieldAttr {
    name: Option<String>,
    /// `#[serde(rename = "…")]`: the JSON key.
    serde_rename: Option<String>,
    /// `#[serde(default)]` (`None`) or `#[serde(default = "path")]`.
    serde_default: Option<Option<Path>>,
    key: Option<String>,
    default: Option<String>,
    rust_default: Option<Expr>,
    optional: bool,
    codec: Option<String>,
    with: Option<Path>,
    rescript_type: Option<String>,
}

#[derive(Default)]
struct VariantAttr {
    alias: Option<String>,
    /// `#[serde(alias = "…")]`: other names it decodes from, `@spice.alias`.
    aliases: Vec<String>,
    /// `#[serde(other)]`: what an unknown name decodes to, `@spice.other`.
    other: bool,
    /// `#[serde(untagged)]`: written as its one value alone,
    /// `@spice.untagged`.
    untagged: bool,
}

/// serde's `rename_all` rules.
#[derive(Clone, Copy)]
enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl RenameRule {
    fn parse(value: &str, span: Span) -> Result<Self> {
        Ok(match value {
            "lowercase" => Self::Lower,
            "UPPERCASE" => Self::Upper,
            "PascalCase" => Self::Pascal,
            "camelCase" => Self::Camel,
            "snake_case" => Self::Snake,
            "SCREAMING_SNAKE_CASE" => Self::ScreamingSnake,
            "kebab-case" => Self::Kebab,
            "SCREAMING-KEBAB-CASE" => Self::ScreamingKebab,
            _ => return Err(Error::new(span, format!("unknown rename rule {value:?}"))),
        })
    }

    /// A `snake_case` field name, as serde renames it.
    fn field(self, name: &str) -> String {
        match self {
            Self::Lower | Self::Snake => name.to_owned(),
            Self::Upper | Self::ScreamingSnake => name.to_ascii_uppercase(),
            Self::Pascal => {
                let camel = camel(name);
                let mut chars = camel.chars();

                chars
                    .next()
                    .map(|c| c.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
            Self::Camel => camel(name),
            Self::Kebab => name.replace('_', "-"),
            Self::ScreamingKebab => name.to_ascii_uppercase().replace('_', "-"),
        }
    }

    /// A `PascalCase` variant name, as serde renames it.
    fn variant(self, name: &str) -> String {
        let snake = || {
            let mut out = String::new();

            for (i, c) in name.char_indices() {
                if i > 0 && c.is_uppercase() {
                    out.push('_');
                }

                out.extend(c.to_lowercase());
            }

            out
        };

        match self {
            Self::Pascal => name.to_owned(),
            Self::Lower => name.to_ascii_lowercase(),
            Self::Upper => name.to_ascii_uppercase(),
            Self::Camel => lower_first(name),
            Self::Snake => snake(),
            Self::ScreamingSnake => snake().to_ascii_uppercase(),
            Self::Kebab => snake().replace('_', "-"),
            Self::ScreamingKebab => snake().to_ascii_uppercase().replace('_', "-"),
        }
    }
}

/// The `#[serde(…)]` items of `attrs`; serde keys rescript-rs doesn't
/// read are left to serde.
fn serde_metas(attrs: &[Attribute]) -> Result<Vec<syn::Meta>> {
    use syn::{punctuated::Punctuated, Meta, Token};

    let mut out = vec![];

    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        out.extend(attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?);
    }

    Ok(out)
}

/// The string of `key = "value"`.
fn meta_string(meta: &syn::Meta) -> Result<String> {
    match meta {
        syn::Meta::NameValue(syn::MetaNameValue {
            value:
                Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }),
            ..
        }) => Ok(s.value()),
        _ => Err(Error::new(
            meta.span(),
            "rescript-rs reads this serde attribute only as key = \"value\"",
        )),
    }
}

fn rescript_attrs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|a| a.path().is_ident("rescript"))
}

/// Whether a container's `#[rescript(…)]` asks for a codec.
pub fn wants_codec(attrs: &[Attribute]) -> bool {
    use syn::{punctuated::Punctuated, Meta, Token};

    rescript_attrs(attrs).any(|attr| {
        attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|metas| metas.iter().any(|meta| meta.path().is_ident("codec")))
    })
}

fn string(meta: &syn::meta::ParseNestedMeta) -> Result<String> {
    Ok(meta.value()?.parse::<LitStr>()?.value())
}

impl ContainerAttr {
    fn parse(attrs: &[Attribute], is_enum: bool) -> Result<Self> {
        let mut out = Self::default();

        for meta in serde_metas(attrs)? {
            let path = meta.path();

            if path.is_ident("rename_all") {
                let rule = RenameRule::parse(&meta_string(&meta)?, meta.span())?;

                if is_enum {
                    out.rename_variants = Some(rule);
                } else {
                    out.rename_fields = Some(rule);
                }
            } else if path.is_ident("rename_all_fields") {
                out.rename_fields = Some(RenameRule::parse(&meta_string(&meta)?, meta.span())?);
            } else if path.is_ident("tag") {
                out.tag = Some(meta_string(&meta)?);
            } else if path.is_ident("content") || path.is_ident("untagged") {
                return Err(Error::new(
                    meta.span(),
                    "spice has no adjacently tagged or untagged enums",
                ));
            }
        }

        for attr in rescript_attrs(attrs) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("codec") {
                } else if meta.path.is_ident("module") {
                    out.module = Some(string(&meta)?);
                } else if meta.path.is_ident("name") {
                    out.name = Some(string(&meta)?);
                } else if meta.path.is_ident("gen_type") {
                    out.gen_type = true;
                } else if meta.path.is_ident("attrs") {
                    out.attrs = Some(string(&meta)?);
                } else if meta.path.is_ident("decode_only") {
                    out.only = Some("@spice.decode");
                } else if meta.path.is_ident("encode_only") {
                    out.only = Some("@spice.encode");
                } else if meta.path.is_ident("legacy") {
                    out.legacy = true;
                } else if meta.path.is_ident("legacy_write") {
                    out.legacy = true;
                    out.legacy_write = true;
                } else if meta.path.is_ident("poly") {
                    out.poly = true;
                } else if meta.path.is_ident("runtime_names") {
                    out.runtime_names = true;
                } else if meta.path.is_ident("tag") {
                    out.tag = Some(string(&meta)?);
                } else if meta.path.is_ident("rename_all") {
                    let value = string(&meta)?;

                    if value != "camelCase" {
                        return Err(meta.error("only rename_all = \"camelCase\" is supported"));
                    }

                    out.rename_fields = Some(RenameRule::Camel);
                } else if meta.path.is_ident("crate") {
                    out.krate = Some(meta.value()?.parse::<LitStr>()?.parse()?);
                } else if meta.path.is_ident("fallback") {
                    out.fallback = Some(meta.value()?.parse::<LitStr>()?.parse()?);
                } else {
                    return Err(meta.error("unknown rescript attribute with codec"));
                }

                Ok(())
            })?;
        }

        Ok(out)
    }
}

impl FieldAttr {
    fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut out = Self::default();

        for meta in serde_metas(attrs)? {
            let path = meta.path();

            if path.is_ident("rename") {
                out.serde_rename = Some(meta_string(&meta)?);
            } else if path.is_ident("default") {
                out.serde_default = Some(match &meta {
                    syn::Meta::Path(_) => None,
                    meta => Some(syn::parse_str(&meta_string(meta)?)?),
                });
            } else if path.is_ident("flatten") || path.is_ident("skip") {
                return Err(Error::new(
                    meta.span(),
                    "spice can't flatten or skip a field",
                ));
            }
        }

        for attr in rescript_attrs(attrs) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    out.name = Some(string(&meta)?);
                } else if meta.path.is_ident("key") {
                    out.key = Some(string(&meta)?);
                } else if meta.path.is_ident("default") {
                    out.default = Some(string(&meta)?);
                } else if meta.path.is_ident("rust_default") {
                    out.rust_default = Some(meta.value()?.parse::<LitStr>()?.parse()?);
                } else if meta.path.is_ident("optional") {
                    out.optional = true;
                } else if meta.path.is_ident("codec") {
                    out.codec = Some(string(&meta)?);
                } else if meta.path.is_ident("with") {
                    out.with = Some(meta.value()?.parse::<LitStr>()?.parse()?);
                } else if meta.path.is_ident("rescript_type") {
                    out.rescript_type = Some(string(&meta)?);
                } else {
                    return Err(meta.error("unknown rescript field attribute with codec"));
                }

                Ok(())
            })?;
        }

        Ok(out)
    }
}

impl VariantAttr {
    fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut out = Self::default();

        for meta in serde_metas(attrs)? {
            if meta.path().is_ident("rename") {
                out.alias = Some(meta_string(&meta)?);
            } else if meta.path().is_ident("alias") {
                out.aliases.push(meta_string(&meta)?);
            } else if meta.path().is_ident("other") {
                out.other = true;
            } else if meta.path().is_ident("untagged") {
                out.untagged = true;
            }
        }

        for attr in rescript_attrs(attrs) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    out.alias = Some(meta.value()?.parse::<LitStr>()?.value());
                } else {
                    return Err(meta.error("unknown rescript variant attribute with codec"));
                }

                Ok(())
            })?;
        }

        Ok(out)
    }
}

fn docs(attrs: &[Attribute], indent: &str) -> String {
    let mut out = String::new();

    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }

        if let syn::Meta::NameValue(syn::MetaNameValue {
            value:
                Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }),
            ..
        }) = &attr.meta
        {
            let line = s.value();
            let line = line.strip_prefix(' ').unwrap_or(&line);

            out.push_str(&format!(
                "{indent}//{}{line}\n",
                if line.is_empty() { "" } else { " " }
            ));
        }
    }

    out
}

fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;

    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }

    out
}

fn lower_first(name: &str) -> String {
    let mut chars = name.chars();

    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `T` when `ty` is `Option<T>`.
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };

    let last = path.path.segments.last()?;

    if last.ident != "Option" {
        return None;
    }

    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };

    match args.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

/// The Rust value of `@spice.default(expr)` for a field of type `ty`.
fn rust_default(rescript: &str, ty: &Type, span: Span) -> Result<TokenStream> {
    let text = rescript.trim();

    if text == "[]" {
        return Ok(quote!(::std::default::Default::default()));
    }

    if text == "None" {
        return Ok(quote!(::std::option::Option::None));
    }

    if text == "true" || text == "false" {
        let value: syn::LitBool = syn::parse_str(text)?;

        return Ok(quote!(#value));
    }

    if text.starts_with('"') {
        let value: LitStr = syn::parse_str(text)?;

        return Ok(quote!(::std::string::String::from(#value)));
    }

    if text.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        let value: Expr = syn::parse_str(text)?;

        return Ok(quote!((#value) as _));
    }

    if let Some(inner) = text.strip_prefix("Some(").and_then(|t| t.strip_suffix(')')) {
        let inner_ty = option_inner(ty).ok_or_else(|| {
            Error::new(
                span,
                "a Some(…) default needs an Option field; use rust_default",
            )
        })?;
        let value = rust_default(inner, inner_ty, span)?;

        return Ok(quote!(::std::option::Option::Some(#value)));
    }

    if text.starts_with(|c: char| c.is_ascii_uppercase())
        && text.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        let constructor = Ident::new(text, span);

        return Ok(quote!(<#ty>::#constructor));
    }

    Err(Error::new(
        span,
        format!("cannot read the ReScript default `{rescript}` as Rust; add rust_default = \"…\""),
    ))
}

struct ParsedField {
    ident: Ident,
    ty: Type,
    rescript_name: String,
    key: String,
    attr: FieldAttr,
    docs: String,
}

fn parse_fields(fields: &syn::FieldsNamed, rename: Option<RenameRule>) -> Result<Vec<ParsedField>> {
    fields
        .named
        .iter()
        .map(|field| {
            let ident = field.ident.clone().expect("named field");
            let attr = FieldAttr::parse(&field.attrs)?;
            let raw = ident.unraw().to_string();
            let json_name = attr
                .serde_rename
                .clone()
                .or_else(|| attr.name.clone())
                .unwrap_or_else(|| rename.map_or_else(|| raw.clone(), |rule| rule.field(&raw)));
            let rescript_name = attr.name.clone().unwrap_or_else(|| {
                let identifier = json_name
                    .starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
                    && json_name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_');

                match identifier {
                    true if RESCRIPT_KEYWORDS.contains(&json_name.as_str()) => {
                        format!("{json_name}_")
                    }
                    true => json_name.clone(),
                    false => raw.clone(),
                }
            });

            if RESCRIPT_KEYWORDS.contains(&rescript_name.as_str()) {
                return Err(Error::new(
                    field.span(),
                    format!(
                        "`{rescript_name}` is a ReScript keyword; add name = \"{rescript_name}_\""
                    ),
                ));
            }

            let key = attr.key.clone().unwrap_or(json_name);

            if PROTOTYPE_KEYS.contains(&key.as_str()) {
                return Err(Error::new(
                    field.span(),
                    format!("spice would read `{key}` from Object.prototype when it is missing"),
                ));
            }

            if attr.serde_default.is_some() && attr.default.is_some() {
                return Err(Error::new(
                    field.span(),
                    "a field has a serde default or a ReScript one, not both",
                ));
            }

            let omittable = option_inner(&field.ty).is_some();

            if attr.optional && !omittable {
                return Err(Error::new(field.span(), "an optional field is an Option"));
            }

            if omittable && (attr.with.is_some() || attr.codec.is_some()) {
                return Err(Error::new(
                    field.span(),
                    "spice ignores a codec on an option field; put it on a non-option field",
                ));
            }

            if attr.with.is_some() != attr.codec.is_some() {
                return Err(Error::new(
                    field.span(),
                    "codec (the ReScript codec) and with (the Rust module) go together",
                ));
            }

            Ok(ParsedField {
                docs: docs(&field.attrs, "  "),
                ident,
                ty: field.ty.clone(),
                rescript_name,
                key,
                attr,
            })
        })
        .collect()
}

/// The local a decoded field is bound to, which can't shadow the
/// decoder's own (`json`, `fields`, `payload`, …).
fn field_var(ident: &Ident) -> Ident {
    format_ident!("spice_field_{}", ident.unraw())
}

/// A polymorphic variant constructor, `#Name`, or `#"name"` when the name
/// isn't an identifier.
fn poly_tag(name: &str) -> String {
    let identifier = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');

    match identifier {
        true => format!("#{name}"),
        false => format!("#{name:?}"),
    }
}

/// The JSON name of a `@spice.serde` constructor.
fn serde_name(v: &ParsedVariant) -> String {
    v.alias.clone().unwrap_or_else(|| v.name.clone())
}

fn validate_serde(container: &ContainerAttr, variants: &[ParsedVariant]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let others: Vec<_> = variants.iter().filter(|v| v.other).collect();

    if let [_, second, ..] = others.as_slice() {
        return Err(Error::new(
            second.variant.span(),
            "Only one variant can be #[serde(other)]",
        ));
    }

    for v in variants {
        let span = v.variant.span();

        if v.other && !matches!(v.variant.fields, Fields::Unit) {
            return Err(Error::new(span, "#[serde(other)] needs a unit variant"));
        }

        if v.untagged {
            if !matches!(&v.variant.fields, Fields::Unnamed(f) if f.unnamed.len() == 1) {
                return Err(Error::new(
                    span,
                    "#[serde(untagged)] needs a variant with one value",
                ));
            }

            if !v.aliases.is_empty() {
                return Err(Error::new(span, "An untagged variant has no name to alias"));
            }

            if !others.is_empty() {
                return Err(Error::new(
                    span,
                    "#[serde(other)] and #[serde(untagged)] can't be combined",
                ));
            }

            continue;
        }

        for name in std::iter::once(serde_name(v)).chain(v.aliases.iter().cloned()) {
            if !seen.insert(name.clone()) {
                return Err(Error::new(
                    span,
                    format!("Two constructors are both named {name} in JSON"),
                ));
            }
        }

        let Some(tag) = &container.tag else {
            continue;
        };

        match &v.variant.fields {
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() > 1 => {
                return Err(Error::new(
                    span,
                    "An internally tagged (tag = …) serde constructor can't have several payload \
                     values, like serde's #[serde(tag = …)]; use named fields, or one value that \
                     encodes to an object",
                ));
            }
            Fields::Named(_) if v.fields.iter().flatten().any(|f| &f.key == tag) => {
                return Err(Error::new(
                    span,
                    format!("A payload field is keyed like the tag {tag}"),
                ));
            }
            _ => {}
        }
    }

    Ok(())
}

/// The decoder and encoder of a `@spice.serde` variant; `array_decode`
/// reads the default spice array from `items`, given `legacy`.
fn derive_serde(
    gen: &Gen,
    container: &ContainerAttr,
    variants: &[ParsedVariant],
    array_decode: TokenStream,
) -> Result<(TokenStream, TokenStream)> {
    let spice = gen.spice();
    let tag = container.tag.as_deref();
    let mut names = vec![];
    let mut objects = vec![];
    let mut encodes = vec![];
    let mut untagged = vec![];

    for v in variants.iter().filter(|v| v.untagged) {
        let constructor = &v.variant.ident;
        let Fields::Unnamed(unnamed) = &v.variant.fields else {
            unreachable!("validated");
        };
        let ty = &unnamed.unnamed[0].ty;

        untagged.push(quote! {
            if let Ok(v0) = <#ty as #spice::Spice>::decode::<H>(json) {
                return Ok(Self::#constructor(v0));
            }
        });
        encodes.push(quote!(Self::#constructor(v0) => #spice::Spice::encode::<H>(v0),));
    }

    let has_other = variants.iter().any(|v| v.other);

    for v in variants.iter().filter(|v| !v.untagged) {
        let constructor = &v.variant.ident;
        let name = serde_name(v);
        let name = std::iter::once(name.clone())
            .chain(v.aliases.iter().cloned())
            .map(|n| quote!(#n))
            .reduce(|a, b| quote!(#a | #b))
            .expect("a name");
        let name_str = serde_name(v);

        // As serde: a bare string naming a variant with data is an error,
        // not the `#[serde(other)]` variant.
        if has_other && !matches!(v.variant.fields, Fields::Unit) {
            names.push(quote!(#name => Err(#spice::serde::unknown(json)),));
        }
        let json_name = quote!(#spice::Value::String(::std::string::String::from(#name_str)));
        let tag_entry = tag.map(|tag| {
            quote!((::std::string::String::from(#tag), ::std::option::Option::Some(#json_name)))
        });
        let wrap = |value: TokenStream| quote!(#spice::object([(::std::string::String::from(#name_str), ::std::option::Option::Some(#value))]));

        match &v.variant.fields {
            Fields::Unit => {
                names.push(quote!(#name => Ok(Self::#constructor),));
                objects.push(match tag {
                    Some(_) => quote!(#name => Ok(Self::#constructor),),
                    None => quote! {
                        #name => #spice::serde::at(
                            #spice::serde::unit::<H>(payload).map(|()| Self::#constructor),
                            name,
                        ),
                    },
                });
                encodes.push(match &tag_entry {
                    Some(entry) => quote!(Self::#constructor => #spice::object([#entry]),),
                    None => quote!(Self::#constructor => #json_name,),
                });
            }
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let ty = &unnamed.unnamed[0].ty;
                let encoded = quote!(#spice::Spice::encode::<H>(v0));

                match tag {
                    Some(tag) => {
                        objects.push(quote! {
                            #name => {
                                let payload = #spice::serde::untagged(&map, #tag);

                                <#ty as #spice::Spice>::decode::<H>(&payload).map(Self::#constructor)
                            }
                        });
                        encodes.push(quote! {
                            Self::#constructor(v0) => #spice::serde::tagged_object(#tag, #name_str, #encoded),
                        });
                    }
                    None => {
                        objects.push(quote! {
                            #name => #spice::serde::at(
                                <#ty as #spice::Spice>::decode::<H>(payload).map(Self::#constructor),
                                name,
                            ),
                        });
                        let value = wrap(encoded);

                        encodes.push(quote!(Self::#constructor(v0) => #value,));
                    }
                }
            }
            Fields::Unnamed(unnamed) => {
                let count = unnamed.unnamed.len();
                let vars: Vec<Ident> = (0..count).map(|i| format_ident!("v{i}")).collect();
                let decodes = unnamed.unnamed.iter().enumerate().map(|(i, f)| {
                    let ty = &f.ty;

                    quote!(#spice::args::decode::<H, #ty>(&items[#i])?)
                });
                let finishes = vars
                    .iter()
                    .enumerate()
                    .map(|(i, var)| quote!(#spice::args::finish(#var, #i)?));
                let value = wrap(
                    quote!(#spice::Value::Array(vec![#(#spice::Spice::encode::<H>(#vars)),*])),
                );

                objects.push(quote! {
                    #name => #spice::serde::at(
                        (|| {
                            let items = #spice::serde::args::<H>(payload, #count)?;
                            let (#(#vars,)*) = (#(#decodes,)*);

                            Ok(Self::#constructor(#(#finishes),*))
                        })(),
                        name,
                    ),
                });
                encodes.push(quote!(Self::#constructor(#(#vars),*) => #value,));
            }
            Fields::Named(_) => {
                let fields = v.fields.as_ref().expect("named");
                let decode_fields = gen.decode_fields(fields, quote!(json))?;
                let idents: Vec<&Ident> = fields.iter().map(|f| &f.ident).collect();
                let vars: Vec<Ident> = fields.iter().map(|f| field_var(&f.ident)).collect();
                let built = quote!(Ok(Self::#constructor { #(#idents: #vars),* }));

                match &tag_entry {
                    Some(entry) => {
                        let object =
                            gen.encode_fields_after(Some(entry.clone()), fields, |f| quote!(#f));

                        objects.push(quote! {
                            #name => (|| {
                                let fields: &#spice::Map<::std::string::String, #spice::Value> = &map;

                                #decode_fields

                                #built
                            })(),
                        });
                        encodes.push(quote!(Self::#constructor { #(#idents),* } => #object,));
                    }
                    None => {
                        let value = wrap(gen.encode_fields(fields, |f| quote!(#f)));

                        objects.push(quote! {
                            #name => #spice::serde::at(
                                (|| {
                                    let fields = #spice::field::object::<H>(payload)?;

                                    #decode_fields

                                    #built
                                })(),
                                name,
                            ),
                        });
                        encodes.push(quote!(Self::#constructor { #(#idents),* } => #value,));
                    }
                }
            }
        }
    }

    // An unknown name decodes to the `#[serde(other)]` variant, if any.
    let unknown = match variants.iter().find(|v| v.other) {
        Some(v) => {
            let constructor = &v.variant.ident;

            quote!(Ok(Self::#constructor))
        }
        None => quote!(Err(#spice::serde::unknown(json))),
    };
    let object = match tag {
        Some(tag) => quote! {
            match #spice::serde::tag::<H>(&map, #tag, json)? {
                #(#objects)*
                _ => #unknown,
            }
        },
        None => {
            // As serde: an unknown name keyed to anything but null is an
            // error.
            let unknown_key = match variants.iter().find(|v| v.other) {
                Some(v) => {
                    let constructor = &v.variant.ident;

                    quote! {
                        #spice::serde::at(
                            #spice::serde::unit::<H>(payload).map(|()| Self::#constructor),
                            name,
                        )
                    }
                }
                None => unknown.clone(),
            };

            quote! {
                let (name, payload) = #spice::serde::external(&map, json)?;

                match name {
                    #(#objects)*
                    _ => #unknown_key,
                }
            }
        }
    };
    let legacy = container.legacy;
    let names = if tag.is_none() || legacy {
        quote! {
            match name {
                #(#names)*
                _ => #unknown,
            }
        }
    } else {
        quote!(Err(#spice::serde::not_a_variant(json)))
    };
    let array_decode = if legacy {
        array_decode
    } else {
        quote!(Err(#spice::serde::found_array(json)))
    };
    let decode = quote! {
        match #spice::serde::shape::<H>(json, #legacy)? {
            #spice::serde::Shape::Name(name) => #names,
            #spice::serde::Shape::Object(map) => { #object }
            #spice::serde::Shape::Array(items) => #array_decode,
        }
    };
    // What no name reads decodes as the first untagged variant it is,
    // keeping the named decoder's error otherwise.
    let decode = if untagged.is_empty() {
        decode
    } else {
        quote! {
            let named = (|| -> #spice::Result<Self> { #decode })();

            if named.is_ok() {
                return named;
            }

            #(#untagged)*

            named
        }
    };
    let encode = quote! {
        match self {
            #(#encodes)*
        }
    };

    Ok((decode, encode))
}

/// The Rust value of a `#[serde(default …)]` field.
fn serde_default(field: &ParsedField) -> TokenStream {
    let ty = &field.ty;

    match field.attr.serde_default.as_ref().expect("serde default") {
        Some(path) => quote!(#path()),
        None => quote!(<#ty as ::std::default::Default>::default()),
    }
}

struct Gen {
    krate: Path,
}

impl Gen {
    fn spice(&self) -> TokenStream {
        let krate = &self.krate;

        quote!(#krate::spice)
    }

    /// The ReScript type expression of a field, as a `String` expression.
    fn field_type(&self, field: &ParsedField) -> TokenStream {
        let spice = self.spice();

        if let Some(ty) = &field.attr.rescript_type {
            return quote!(::std::string::String::from(#ty));
        }

        if field.attr.optional {
            let inner = option_inner(&field.ty).expect("checked");

            return quote!(<#inner as #spice::Spice>::rescript_type());
        }

        let ty = &field.ty;

        quote!(<#ty as #spice::Spice>::rescript_type())
    }

    /// `  @spice.key("K") …\n  name?: type,` for one record field.
    fn field_declaration(&self, field: &ParsedField, inline: bool) -> TokenStream {
        let spice = self.spice();
        let mut attrs = vec![];

        if field.key != field.rescript_name {
            let key = format!("@spice.key({:?})", field.key);

            attrs.push(quote!(::std::string::String::from(#key)));
        }

        if let Some(default) = &field.attr.default {
            let default = format!("@spice.default({default})");

            attrs.push(quote!(::std::string::String::from(#default)));
        } else if field.attr.serde_default.is_some() {
            let value = serde_default(field);
            let ty = &field.ty;
            let missing = format!(
                "the default of `{}` has no ReScript value (Spice::rescript_value)",
                field.rescript_name
            );

            attrs.push(quote!(format!(
                "@spice.default({})",
                #spice::Spice::rescript_value(&{ let value: #ty = #value; value }).expect(#missing),
            )));
        }

        let name = format!(
            "{}{}",
            field.rescript_name,
            if field.attr.optional { "?" } else { "" }
        );
        let codec = field
            .attr
            .codec
            .as_ref()
            .map(|c| format!("@spice.codec({c}) "))
            .unwrap_or_default();
        let ty = self.field_type(field);
        let attrs = quote!(::std::vec::Vec::<::std::string::String>::from([#(#attrs),*]).join(" "));

        if inline {
            quote!({
                let attrs: ::std::string::String = #attrs;
                let attrs = if attrs.is_empty() { attrs } else { format!("{attrs} ") };

                format!("{}{}: {}{}", attrs, #name, #codec, #ty)
            })
        } else {
            let docs = &field.docs;

            quote!({
                let attrs: ::std::string::String = #attrs;
                let attrs = if attrs.is_empty() { attrs } else { format!("  {attrs}\n") };

                format!("{}{}  {}: {}{},\n", #docs, attrs, #name, #codec, #ty)
            })
        }
    }

    fn decode_fn(&self, field: &ParsedField) -> TokenStream {
        let spice = self.spice();

        if let Some(with) = &field.attr.with {
            return quote!(#with::decode::<H>);
        }

        match option_inner(&field.ty) {
            Some(inner) => quote!(|json: &#spice::Value| {
                #spice::field::omittable::<H, #inner>(json, <#inner as #spice::Spice>::decode::<H>)
            }),
            None => {
                let ty = &field.ty;

                quote!(<#ty as #spice::Spice>::decode::<H>)
            }
        }
    }

    /// `let name = …?;` for each field, in order, reading from `fields`.
    fn decode_fields(&self, fields: &[ParsedField], record: TokenStream) -> Result<TokenStream> {
        let spice = self.spice();
        let mut out = vec![];

        for field in fields {
            let ident = field_var(&field.ident);
            let key = &field.key;
            let decode = self.decode_fn(field);
            let default = match (&field.attr.rust_default, &field.attr.default) {
                (Some(expr), _) => Some(quote!(#expr)),
                (None, None) if field.attr.serde_default.is_some() => Some(serde_default(field)),
                (None, Some(rescript)) => {
                    Some(rust_default(rescript, &field.ty, field.ident.span())?)
                }
                (None, None) => None,
            };

            out.push(match (default, option_inner(&field.ty)) {
                (Some(default), _) => quote! {
                    let #ident = #spice::field::defaulted(&fields, #key, || #default, #decode)?;
                },
                (None, Some(_)) => quote! {
                    let #ident = #spice::field::optional(&fields, #key, #decode)?;
                },
                (None, None) => quote! {
                    let #ident = #spice::field::required(&fields, #key, #record, #decode)?;
                },
            });
        }

        Ok(quote!(#(#out)*))
    }

    /// `(key, Option<Value>)` entries for `spice::object`.
    fn encode_fields(
        &self,
        fields: &[ParsedField],
        access: impl Fn(&Ident) -> TokenStream,
    ) -> TokenStream {
        self.encode_fields_after(None, fields, access)
    }

    /// [`Self::encode_fields`] after a `(key, value)` entry, e.g. a tag.
    fn encode_fields_after(
        &self,
        leading: Option<TokenStream>,
        fields: &[ParsedField],
        access: impl Fn(&Ident) -> TokenStream,
    ) -> TokenStream {
        let spice = self.spice();
        let entries = leading.into_iter().chain(fields.iter().map(|field| {
            let key = &field.key;
            let value = access(&field.ident);

            if let Some(with) = &field.attr.with {
                return quote!((::std::string::String::from(#key), ::std::option::Option::Some(#with::encode::<H>(#value))));
            }

            match option_inner(&field.ty) {
                Some(_) => quote! {
                    (::std::string::String::from(#key), (#value).as_ref().map(|v| #spice::Spice::encode::<H>(v)))
                },
                None => quote! {
                    (::std::string::String::from(#key), ::std::option::Option::Some(#spice::Spice::encode::<H>(#value)))
                },
            }
        }));

        quote!(#spice::object([#(#entries),*]))
    }
}

pub fn derive(input: DeriveInput) -> Result<TokenStream> {
    let container = ContainerAttr::parse(&input.attrs, matches!(input.data, Data::Enum(_)))?;
    let krate = container
        .krate
        .clone()
        .unwrap_or_else(|| syn::parse_quote!(::rescript_rs));
    let gen = Gen { krate };
    let spice = gen.spice();

    check_tuples(&input.data)?;

    if !input.generics.params.is_empty() {
        return Err(Error::new(
            input.generics.span(),
            "Spice types cannot be generic yet",
        ));
    }

    let ident = &input.ident;
    let name = container
        .name
        .clone()
        .unwrap_or_else(|| lower_first(&ident.unraw().to_string()));
    let qualified = match &container.module {
        Some(module) => format!("{module}.{name}"),
        None => name.clone(),
    };

    let samples = samples(&gen, &input.data)?;
    let rescript_value = rescript_value(&gen, &container, &input.data)?;
    let (body, decode, encode) = match &input.data {
        Data::Struct(_)
            if container.tag.is_some()
                || container.legacy
                || container.poly
                || container.runtime_names =>
        {
            return Err(Error::new(
                input.span(),
                "tag, legacy, poly and runtime_names are for enums",
            ))
        }
        Data::Struct(data) => derive_struct(&gen, &container, data, ident)?,
        Data::Enum(data) => derive_enum(&gen, &container, data)?,
        Data::Union(_) => return Err(Error::new(input.span(), "unions are not supported")),
    };

    let mut header = docs(&input.attrs, "");

    let codec = match &input.data {
        Data::Enum(_) if container.legacy_write => "@spice.serde({legacyWrite: true})",
        Data::Enum(_) if container.legacy => "@spice.serde({legacy: true})",
        Data::Enum(_) => "@spice.serde",
        _ => "@spice",
    };

    header.push_str(&match container.only {
        Some(only) if codec == "@spice" => only.to_string(),
        Some(only) => format!("{codec} {only}"),
        None => codec.to_string(),
    });

    if let Some(tag) = &container.tag {
        header.push_str(&format!(" @tag({tag:?})"));
    }

    if container.gen_type {
        header.push_str(" @genType");
    }

    if let Some(attrs) = &container.attrs {
        header.push(' ');
        header.push_str(attrs);
    }

    let rec = if refers_to_itself(&input.data, ident) {
        "rec "
    } else {
        ""
    };

    let type_line = format!("\ntype {rec}{name} =");

    // `@spice.fallback(…)` with the fallback value as a ReScript literal.
    let fallback_attr = match &container.fallback {
        Some(path) => {
            let message = format!("{} has no ReScript value", quote!(#path));

            quote!(format!(
                " @spice.fallback({})",
                #spice::Spice::rescript_value(&#path()).expect(#message),
            ))
        }
        None => quote!(::std::string::String::new()),
    };
    let decode = match &container.fallback {
        Some(path) => quote! {
            let decoded = (|| -> #spice::Result<Self> { #decode })();

            decoded.or_else(|_| Ok(#path()))
        },
        None => decode,
    };
    let declare = container.module.as_ref().map(|module| {
        quote! {
            #[automatically_derived]
            impl #spice::Declare for #ident {
                fn module() -> &'static str {
                    #module
                }

                fn declaration() -> ::std::string::String {
                    format!("{}{}{}{}", #header, #fallback_attr, #type_line, #body)
                }
            }
        }
    });

    Ok(quote! {
        #[automatically_derived]
        impl #spice::Spice for #ident {
            fn rescript_type() -> ::std::string::String {
                ::std::string::String::from(#qualified)
            }

            fn samples() -> ::std::vec::Vec<Self> {
                #spice::samples_once(|| { #samples })
            }

            fn decode<H: #spice::Host>(json: &#spice::Value) -> #spice::Result<Self> {
                #decode
            }

            fn encode<H: #spice::Host>(&self) -> #spice::Value {
                #encode
            }

            #[allow(unreachable_code, clippy::needless_return)]
            fn rescript_value(&self) -> ::std::option::Option<::std::string::String> {
                #rescript_value
            }
        }

        #declare
    })
}

/// `Spice::rescript_value`: the value as a ReScript literal.
fn rescript_value(gen: &Gen, container: &ContainerAttr, data: &Data) -> Result<TokenStream> {
    let spice = gen.spice();
    // `name: value` for each field, or no value if any field has none.
    let record = |fields: &[ParsedField], access: &dyn Fn(&Ident) -> TokenStream| {
        let parts = fields.iter().map(|field| {
            let name = &field.rescript_name;
            let value = access(&field.ident);

            if field.attr.with.is_some() {
                return quote!(return ::std::option::Option::None;);
            }

            if field.attr.optional {
                quote! {
                    if let ::std::option::Option::Some(value) = #value {
                        parts.push(format!("{}: {}", #name, #spice::Spice::rescript_value(value)?));
                    }
                }
            } else {
                quote!(parts.push(format!("{}: {}", #name, #spice::Spice::rescript_value(#value)?));)
            }
        });

        quote!({
            let mut parts: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();

            #(#parts)*

            parts.join(", ")
        })
    };

    Ok(match data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => {
                let fields = parse_fields(named, container.rename_fields)?;
                let body = record(&fields, &|field| quote!(&self.#field));

                // The first label qualified by the record's module, `{M.a: …}`,
                // names the type: `rescript format` breaks `({…}: M.t)` in an
                // attribute.
                quote! {
                    let body = #body;

                    if body.is_empty() {
                        return ::std::option::Option::None;
                    }

                    ::std::option::Option::Some(format!(
                        "{{{}{}}}",
                        #spice::rescript_scope(&<Self as #spice::Spice>::rescript_type()),
                        body,
                    ))
                }
            }
            Fields::Unnamed(_) => quote!(#spice::Spice::rescript_value(&self.0)),
            Fields::Unit => quote!(::std::option::Option::None),
        },
        Data::Enum(data) => {
            let mut arms = vec![];

            for variant in &data.variants {
                let constructor = &variant.ident;
                let name = constructor.unraw().to_string();
                let head = if container.poly {
                    let tag = match container.runtime_names {
                        true => {
                            let attr = VariantAttr::parse(&variant.attrs)?;
                            let json = attr
                                .alias
                                .or_else(|| {
                                    container.rename_variants.map(|rule| rule.variant(&name))
                                })
                                .unwrap_or(name);

                            poly_tag(&json)
                        }
                        false => poly_tag(&name),
                    };

                    quote!(::std::string::String::from(#tag))
                } else {
                    quote!(format!(
                        "{}{}",
                        #spice::rescript_scope(&<Self as #spice::Spice>::rescript_type()),
                        #name,
                    ))
                };

                arms.push(match &variant.fields {
                    Fields::Unit => quote!(Self::#constructor => ::std::option::Option::Some(#head),),
                    Fields::Unnamed(unnamed) => {
                        let vars: Vec<Ident> =
                            (0..unnamed.unnamed.len()).map(|i| format_ident!("v{i}")).collect();

                        quote! {
                            Self::#constructor(#(#vars),*) => {
                                let args = [#(#spice::Spice::rescript_value(#vars)?),*];

                                ::std::option::Option::Some(format!("{}({})", #head, args.join(", ")))
                            }
                        }
                    }
                    Fields::Named(named) => {
                        let fields = parse_fields(named, container.rename_fields)?;
                        let idents: Vec<&Ident> = fields.iter().map(|f| &f.ident).collect();
                        let body = record(&fields, &|field| quote!(#field));

                        quote! {
                            Self::#constructor { #(#idents),* } => {
                                let body = #body;

                                ::std::option::Option::Some(format!("{}({{{}}})", #head, body))
                            }
                        }
                    }
                });
            }

            quote!(match self { #(#arms)* })
        }
        Data::Union(_) => quote!(::std::option::Option::None),
    })
}

/// `rescript_rs::spice::MAX_TUPLE`.
const MAX_TUPLE: usize = 16;

/// Rejects a tuple longer than the runtime has a codec for, anywhere in a
/// field's type, with a clearer error than a missing trait impl.
fn check_tuples(data: &Data) -> Result<()> {
    fn check(ty: &Type) -> Result<()> {
        match ty {
            Type::Tuple(tuple) if tuple.elems.len() > MAX_TUPLE => Err(Error::new(
                tuple.span(),
                format!(
                    "a Spice tuple has at most {MAX_TUPLE} values (this one has {}); use a record",
                    tuple.elems.len()
                ),
            )),
            Type::Tuple(tuple) => tuple.elems.iter().try_for_each(check),
            Type::Paren(inner) => check(&inner.elem),
            Type::Group(inner) => check(&inner.elem),
            Type::Path(path) => {
                path.path
                    .segments
                    .iter()
                    .try_for_each(|segment| match &segment.arguments {
                        PathArguments::AngleBracketed(args) => {
                            args.args.iter().try_for_each(|arg| match arg {
                                GenericArgument::Type(ty) => check(ty),
                                _ => Ok(()),
                            })
                        }
                        _ => Ok(()),
                    })
            }
            _ => Ok(()),
        }
    }

    match data {
        Data::Struct(data) => data.fields.iter().try_for_each(|f| check(&f.ty)),
        Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|v| v.fields.iter())
            .try_for_each(|f| check(&f.ty)),
        Data::Union(_) => Ok(()),
    }
}

/// Whether a field's type names the type itself (`Self` or its name),
/// which ReScript declares with `type rec`.
fn refers_to_itself(data: &Data, ident: &Ident) -> bool {
    fn mentions(tokens: TokenStream, ident: &Ident) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(name) => name == *ident || name == "Self",
            proc_macro2::TokenTree::Group(group) => mentions(group.stream(), ident),
            _ => false,
        })
    }

    let fields: Vec<&syn::Field> = match data {
        Data::Struct(data) => data.fields.iter().collect(),
        Data::Enum(data) => data.variants.iter().flat_map(|v| v.fields.iter()).collect(),
        Data::Union(_) => vec![],
    };

    fields.iter().any(|field| mentions(quote!(#field), ident))
}

type Derived = (TokenStream, TokenStream, TokenStream);

fn derive_struct(
    gen: &Gen,
    container: &ContainerAttr,
    data: &syn::DataStruct,
    ident: &Ident,
) -> Result<Derived> {
    let spice = gen.spice();

    match &data.fields {
        Fields::Named(named) => {
            let fields = parse_fields(named, container.rename_fields)?;
            let lines = fields.iter().map(|f| gen.field_declaration(f, false));
            let body = quote!(format!(" {{\n{}}}", [#(#lines),*].concat()));
            let decode_fields = gen.decode_fields(&fields, quote!(json))?;
            let idents = fields.iter().map(|f| &f.ident);
            let vars = fields.iter().map(|f| field_var(&f.ident));
            let decode = quote! {
                let fields = #spice::field::object::<H>(json)?;

                #decode_fields

                Ok(#ident { #(#idents: #vars),* })
            };
            let encode = gen.encode_fields(&fields, |field| quote!(&self.#field));

            Ok((body, decode, encode))
        }
        Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
            let ty = &unnamed.unnamed[0].ty;
            let body = quote!(format!(" {}", <#ty as #spice::Spice>::rescript_type()));
            let decode = quote!(<#ty as #spice::Spice>::decode::<H>(json).map(#ident));
            let encode = quote!(#spice::Spice::encode::<H>(&self.0));

            Ok((body, decode, encode))
        }
        _ => Err(Error::new(
            data.fields.span(),
            "a Spice struct has named fields, or is a newtype for a type alias",
        )),
    }
}

struct ParsedVariant<'a> {
    variant: &'a syn::Variant,
    /// The ReScript constructor, which is also its name in JSON.
    name: String,
    alias: Option<String>,
    aliases: Vec<String>,
    other: bool,
    untagged: bool,
    fields: Option<Vec<ParsedField>>,
}

fn derive_enum(gen: &Gen, container: &ContainerAttr, data: &syn::DataEnum) -> Result<Derived> {
    let spice = gen.spice();
    let variants = data
        .variants
        .iter()
        .map(|variant| {
            let name = variant.ident.unraw().to_string();
            let attr = VariantAttr::parse(&variant.attrs)?;
            let alias = attr.alias.clone().or_else(|| {
                container
                    .rename_variants
                    .map(|rule| rule.variant(&name))
                    .filter(|renamed| *renamed != name)
            });
            // A polymorphic variant's runtime value is its tag: with
            // runtime_names, the tag is the JSON name.
            let (name, alias) = match (container.poly && container.runtime_names, alias) {
                (true, Some(json)) => (json, None),
                (_, alias) => (name, alias),
            };
            let fields = match &variant.fields {
                Fields::Named(_) if container.poly => {
                    return Err(Error::new(
                        variant.span(),
                        "a polymorphic variant constructor can't have named fields",
                    ))
                }
                Fields::Named(named) => Some(parse_fields(named, container.rename_fields)?),
                _ => None,
            };

            Ok(ParsedVariant {
                variant,
                name,
                alias,
                aliases: attr.aliases,
                other: attr.other,
                untagged: attr.untagged,
                fields,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    validate_serde(container, &variants)?;

    let mut lines = vec![];

    for v in &variants {
        let constructor = match container.poly {
            true => poly_tag(&v.name),
            false => v.name.clone(),
        };
        let docs = docs(&v.variant.attrs, "  ");
        let alias = v
            .alias
            .as_ref()
            .map(|a| match container.runtime_names {
                true => format!("@as({a:?}) "),
                false => format!("@spice.as({a:?}) "),
            })
            .unwrap_or_default();
        let alias = format!(
            "{alias}{}{}{}",
            v.aliases
                .iter()
                .map(|a| format!("@spice.alias({a:?}) "))
                .collect::<String>(),
            if v.other { "@spice.other " } else { "" },
            if v.untagged { "@spice.untagged " } else { "" },
        );

        lines.push(match &v.variant.fields {
            Fields::Unit => quote!(format!("{}  | {}{}\n", #docs, #alias, #constructor)),
            Fields::Unnamed(unnamed) => {
                let types = unnamed.unnamed.iter().map(|f| {
                    let ty = &f.ty;

                    quote!(<#ty as #spice::Spice>::rescript_type())
                });

                quote!(format!("{}  | {}{}({})\n", #docs, #alias, #constructor, [#(#types),*].join(", ")))
            }
            Fields::Named(_) => {
                let fields = v.fields.as_ref().expect("named");
                let decls = fields.iter().map(|f| gen.field_declaration(f, true));

                quote!(format!("{}  | {}{}({{{}}})\n", #docs, #alias, #constructor, [#(#decls),*].join(", ")))
            }
        });
    }

    let body = if container.poly {
        quote!(format!(" [\n{}\n]", [#(#lines),*].concat().trim_end()))
    } else {
        quote!(format!("\n{}", [#(#lines),*].concat().trim_end()))
    };

    let kind = if container.poly {
        quote!(#spice::variant::Kind::Poly)
    } else {
        quote!(#spice::variant::Kind::Variant)
    };
    let mut decode_cases = vec![];
    let mut array_encodes = vec![];

    let constructors: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();

    for v in &variants {
        let constructor = &v.variant.ident;
        let tag = &v.name;
        // The legacy array reads the constructor and its other aliases.
        let tags = std::iter::once(tag)
            .chain(
                v.aliases
                    .iter()
                    .filter(|a| !constructors.contains(&a.as_str())),
            )
            .map(|t| quote!(::std::option::Option::Some(#t)))
            .reduce(|a, b| quote!(#a | #b))
            .expect("a tag");
        let json_tag = quote!(#spice::Value::String(::std::string::String::from(#tag)));

        match &v.variant.fields {
            Fields::Unit => {
                array_encodes.push(quote! {
                    Self::#constructor => #spice::Value::Array(vec![#json_tag]),
                });
                decode_cases.push(quote! {
                    #tags => {
                        #spice::variant::arity_of(#kind, &items, 0, json)?;

                        Ok(Self::#constructor)
                    }
                });
            }
            Fields::Unnamed(unnamed) => {
                let count = unnamed.unnamed.len();
                let vars: Vec<Ident> = (0..count).map(|i| format_ident!("v{i}")).collect();
                let decodes = unnamed.unnamed.iter().enumerate().map(|(i, f)| {
                    let ty = &f.ty;
                    let index = i + 1;

                    quote!(#spice::args::decode::<H, #ty>(&items[#index])?)
                });
                let finishes = vars.iter().enumerate().map(|(i, var)| {
                    let index = i + 1;

                    quote!(#spice::args::finish(#var, #index)?)
                });

                array_encodes.push(quote! {
                    Self::#constructor(#(#vars),*) => #spice::Value::Array(vec![
                        #json_tag,
                        #(#spice::Spice::encode::<H>(#vars)),*
                    ]),
                });

                decode_cases.push(quote! {
                    #tags => {
                        #spice::variant::arity_of(#kind, &items, #count, json)?;

                        let (#(#vars,)*) = (#(#decodes,)*);

                        Ok(Self::#constructor(#(#finishes),*))
                    }
                });
            }
            Fields::Named(_) => {
                let fields = v.fields.as_ref().expect("named");
                let decode_fields = gen.decode_fields(fields, quote!(payload))?;
                let idents: Vec<&Ident> = fields.iter().map(|f| &f.ident).collect();
                let vars: Vec<Ident> = fields.iter().map(|f| field_var(&f.ident)).collect();
                let object = gen.encode_fields(fields, |f| quote!(#f));

                array_encodes.push(quote! {
                    Self::#constructor { #(#idents),* } => #spice::Value::Array(vec![#json_tag, #object]),
                });
                decode_cases.push(quote! {
                    #tags => {
                        #spice::variant::arity_of(#kind, &items, 1, json)?;

                        let payload = &items[1];
                        let decoded = (|| -> #spice::Result<Self> {
                            let fields = #spice::field::object::<H>(payload)?;

                            #decode_fields

                            Ok(Self::#constructor { #(#idents: #vars),* })
                        })();

                        decoded.map_err(|error| error.prefixed("[1]"))
                    }
                });
            }
        }
    }

    let unknown = match variants.iter().find(|v| v.other) {
        // A unit variant: no arguments.
        Some(v) => {
            let constructor = &v.variant.ident;

            quote! {{
                #spice::variant::arity_of(#kind, &items, 0, json)?;

                Ok(Self::#constructor)
            }}
        }
        None => quote!(Err(#spice::variant::unknown_of(#kind, &items))),
    };
    let array_decode = quote! {
        match #spice::variant::tag::<H>(&items) {
            #(#decode_cases)*
            _ => #unknown,
        }
    };

    let (decode, encode) = derive_serde(gen, container, &variants, array_decode)?;
    let encode = if container.legacy_write {
        quote! {
            match self {
                #(#array_encodes)*
            }
        }
    } else {
        encode
    };

    Ok((body, decode, encode))
}

/// The samples of a field's type, `None` when it has none.
fn field_samples(gen: &Gen, ty: &Type, with: Option<&Path>) -> TokenStream {
    let spice = gen.spice();

    match with {
        Some(with) => quote!(#with::samples()),
        None => quote!(<#ty as #spice::Spice>::samples()),
    }
}

/// A record's samples: the first sample of every field, then one more per
/// other sample of a field. `constructor` builds the value from `fields`.
fn record_samples(
    gen: &Gen,
    fields: &[(TokenStream, &Type, Option<&Path>)],
    construct: impl Fn(&[TokenStream]) -> TokenStream,
) -> TokenStream {
    let vars: Vec<Ident> = (0..fields.len()).map(|i| format_ident!("s{i}")).collect();
    let lets = fields.iter().zip(&vars).map(|((_, ty, with), var)| {
        let samples = field_samples(gen, ty, *with);

        quote!(let #var = #samples;)
    });
    let firsts: Vec<TokenStream> = vars.iter().map(|var| quote!(#var[0].clone())).collect();
    let base = construct(&firsts);
    let variations = vars.iter().enumerate().map(|(i, var)| {
        let mut values = firsts.clone();

        values[i] = quote!(sample.clone());

        let value = construct(&values);

        quote! {
            for sample in #var.iter().skip(1) {
                out.push(#value);
            }
        }
    });

    quote! {{
        #(#lets)*

        if [#(#vars.is_empty()),*].into_iter().any(|empty| empty) {
            ::std::vec::Vec::new()
        } else {
            let mut out = vec![#base];

            #(#variations)*

            out
        }
    }}
}

fn samples(gen: &Gen, data: &Data) -> Result<TokenStream> {
    match data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => {
                let fields: Vec<_> = named
                    .named
                    .iter()
                    .map(|f| {
                        let attr = FieldAttr::parse(&f.attrs)?;
                        let ident = f.ident.clone().expect("named");

                        Ok((quote!(#ident), &f.ty, attr.with))
                    })
                    .collect::<Result<_>>()?;
                let refs: Vec<_> = fields
                    .iter()
                    .map(|(i, t, w)| (i.clone(), *t, w.as_ref()))
                    .collect();
                let names: Vec<TokenStream> = fields.iter().map(|(i, _, _)| i.clone()).collect();

                Ok(record_samples(
                    gen,
                    &refs,
                    |values| quote!(Self { #(#names: #values),* }),
                ))
            }
            Fields::Unnamed(unnamed) => {
                let ty = &unnamed.unnamed[0].ty;
                let spice = gen.spice();

                Ok(quote!(<#ty as #spice::Spice>::samples().into_iter().map(Self).collect()))
            }
            Fields::Unit => Ok(quote!(vec![Self])),
        },
        Data::Enum(data) => {
            let mut parts = vec![];

            for variant in &data.variants {
                let constructor = &variant.ident;

                parts.push(match &variant.fields {
                    Fields::Unit => quote!(out.push(Self::#constructor);),
                    Fields::Unnamed(unnamed) => {
                        let fields: Vec<_> = unnamed
                            .unnamed
                            .iter()
                            .map(|f| (quote!(), &f.ty, None))
                            .collect();
                        let samples = record_samples(
                            gen,
                            &fields,
                            |values| quote!(Self::#constructor(#(#values),*)),
                        );

                        quote!(out.extend(#samples);)
                    }
                    Fields::Named(named) => {
                        let parsed: Vec<_> = named
                            .named
                            .iter()
                            .map(|f| {
                                let attr = FieldAttr::parse(&f.attrs)?;
                                let ident = f.ident.clone().expect("named");

                                Ok((ident, &f.ty, attr.with))
                            })
                            .collect::<Result<_>>()?;
                        let fields: Vec<_> = parsed
                            .iter()
                            .map(|(i, t, w)| (quote!(#i), *t, w.as_ref()))
                            .collect();
                        let names: Vec<&Ident> = parsed.iter().map(|(i, _, _)| i).collect();
                        let samples = record_samples(
                            gen,
                            &fields,
                            |values| quote!(Self::#constructor { #(#names: #values),* }),
                        );

                        quote!(out.extend(#samples);)
                    }
                });
            }

            Ok(quote! {
                let mut out = ::std::vec::Vec::new();

                #(#parts)*

                out
            })
        }
        Data::Union(_) => Ok(quote!(::std::vec::Vec::new())),
    }
}
