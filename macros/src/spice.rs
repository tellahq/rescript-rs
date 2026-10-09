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
    "and", "as", "assert", "await", "constraint", "else", "exception", "external", "false", "for",
    "if", "in", "include", "lazy", "let", "module", "mutable", "of", "open", "private", "rec",
    "switch", "true", "try", "type", "when", "while", "with",
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
    unboxed: bool,
    camel_case: bool,
    /// `@spice.decode` or `@spice.encode` instead of `@spice`: ReScript gets
    /// only that function (Rust gets both).
    only: Option<&'static str>,
    /// `@spice.serde`: variants encoded like Rust's serde derive.
    serde: bool,
    /// `@tag(…)` with `serde`: internally tagged.
    tag: Option<String>,
    krate: Option<Path>,
    /// A polymorphic variant, `[#A | #B]`.
    poly: bool,
}

#[derive(Default)]
struct FieldAttr {
    name: Option<String>,
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
    /// The ReScript constructor, when it isn't the Rust one.
    name: Option<String>,
    alias: Option<AliasValue>,
}

#[derive(Clone)]
enum AliasValue {
    String(String),
    Number(f64),
}

fn spice_attrs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|a| a.path().is_ident("spice"))
}

fn string(meta: &syn::meta::ParseNestedMeta) -> Result<String> {
    Ok(meta.value()?.parse::<LitStr>()?.value())
}

impl ContainerAttr {
    fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut out = Self::default();

        for attr in spice_attrs(attrs) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("module") {
                    out.module = Some(string(&meta)?);
                } else if meta.path.is_ident("name") {
                    out.name = Some(string(&meta)?);
                } else if meta.path.is_ident("attrs") {
                    out.attrs = Some(string(&meta)?);
                } else if meta.path.is_ident("decode_only") {
                    out.only = Some("@spice.decode");
                } else if meta.path.is_ident("encode_only") {
                    out.only = Some("@spice.encode");
                } else if meta.path.is_ident("unboxed") {
                    out.unboxed = true;
                } else if meta.path.is_ident("serde") {
                    out.serde = true;
                } else if meta.path.is_ident("poly") {
                    out.poly = true;
                } else if meta.path.is_ident("tag") {
                    out.tag = Some(string(&meta)?);
                } else if meta.path.is_ident("rename_all") {
                    let value = string(&meta)?;

                    if value != "camelCase" {
                        return Err(meta.error("only rename_all = \"camelCase\" is supported"));
                    }

                    out.camel_case = true;
                } else if meta.path.is_ident("crate") {
                    out.krate = Some(meta.value()?.parse::<LitStr>()?.parse()?);
                } else {
                    return Err(meta.error("unknown spice attribute"));
                }

                Ok(())
            })?;
        }

        if out.tag.is_some() && !out.serde {
            return Err(Error::new(
                Span::call_site(),
                "tag needs serde: #[spice(serde, tag = \"…\")]",
            ));
        }

        if out.poly && out.unboxed {
            return Err(Error::new(
                Span::call_site(),
                "a polymorphic variant can't be unboxed",
            ));
        }

        if out.serde && out.unboxed {
            return Err(Error::new(
                Span::call_site(),
                "@spice.serde can't be combined with @unboxed",
            ));
        }

        Ok(out)
    }
}

impl FieldAttr {
    fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut out = Self::default();

        for attr in spice_attrs(attrs) {
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
                    return Err(meta.error("unknown spice field attribute"));
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

        for attr in spice_attrs(attrs) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    out.name = Some(string(&meta)?);
                } else if meta.path.is_ident("alias") {
                    let lit: syn::Lit = meta.value()?.parse()?;

                    out.alias = Some(match lit {
                        syn::Lit::Str(s) => AliasValue::String(s.value()),
                        syn::Lit::Int(i) => AliasValue::Number(i.base10_parse()?),
                        syn::Lit::Float(f) => AliasValue::Number(f.base10_parse()?),
                        _ => return Err(meta.error("alias is a string or a number")),
                    });
                } else {
                    return Err(meta.error("unknown spice variant attribute"));
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

            out.push_str(&format!("{indent}//{}{line}\n", if line.is_empty() { "" } else { " " }));
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
            Error::new(span, "a Some(…) default needs an Option field; use rust_default")
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

fn parse_fields(fields: &syn::FieldsNamed, camel_case: bool) -> Result<Vec<ParsedField>> {
    fields
        .named
        .iter()
        .map(|field| {
            let ident = field.ident.clone().expect("named field");
            let attr = FieldAttr::parse(&field.attrs)?;
            let raw = ident.unraw().to_string();
            let rescript_name = attr
                .name
                .clone()
                .unwrap_or_else(|| if camel_case { camel(&raw) } else { raw });

            if RESCRIPT_KEYWORDS.contains(&rescript_name.as_str()) {
                return Err(Error::new(
                    field.span(),
                    format!("`{rescript_name}` is a ReScript keyword; add name = \"{rescript_name}_\""),
                ));
            }

            let key = attr.key.clone().unwrap_or_else(|| rescript_name.clone());

            if PROTOTYPE_KEYS.contains(&key.as_str()) {
                return Err(Error::new(
                    field.span(),
                    format!("spice would read `{key}` from Object.prototype when it is missing"),
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

/// The JSON name of a `@spice.serde` constructor.
fn serde_name(v: &ParsedVariant) -> String {
    match &v.alias {
        Some(AliasValue::String(name)) => name.clone(),
        _ => v.name.clone(),
    }
}

fn validate_serde(container: &ContainerAttr, variants: &[ParsedVariant]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();

    for v in variants {
        let span = v.variant.span();

        if let Some(AliasValue::Number(_)) = v.alias {
            return Err(Error::new(
                span,
                "@spice.serde needs @spice.as to be a string",
            ));
        }

        let name = serde_name(v);

        if !seen.insert(name.clone()) {
            return Err(Error::new(
                span,
                format!("Two constructors are both named {name} in JSON"),
            ));
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
/// reads the default spice array from `items`.
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

    for v in variants {
        let constructor = &v.variant.ident;
        let name = serde_name(v);
        let json_name = quote!(#spice::Value::String(::std::string::String::from(#name)));
        let tag_entry = tag.map(|tag| {
            quote!((::std::string::String::from(#tag), ::std::option::Option::Some(#json_name)))
        });
        let wrap = |value: TokenStream| quote!(#spice::object([(::std::string::String::from(#name), ::std::option::Option::Some(#value))]));

        match &v.variant.fields {
            Fields::Unit => {
                names.push(quote!(#name => Ok(Self::#constructor),));
                objects.push(match tag {
                    Some(_) => quote!(#name => Ok(Self::#constructor),),
                    None => quote! {
                        #name => #spice::serde::at(
                            #spice::serde::unit::<H>(payload).map(|()| Self::#constructor),
                            #name,
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
                            Self::#constructor(v0) => #spice::serde::tagged_object(#tag, #name, #encoded),
                        });
                    }
                    None => {
                        objects.push(quote! {
                            #name => #spice::serde::at(
                                <#ty as #spice::Spice>::decode::<H>(payload).map(Self::#constructor),
                                #name,
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
                        #name,
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
                                #name,
                            ),
                        });
                        encodes.push(quote!(Self::#constructor { #(#idents),* } => #value,));
                    }
                }
            }
        }
    }

    let object = match tag {
        Some(tag) => quote! {
            match #spice::serde::tag::<H>(&map, #tag, json)? {
                #(#objects)*
                _ => Err(#spice::serde::unknown(json)),
            }
        },
        None => quote! {
            let (name, payload) = #spice::serde::external(&map, json)?;

            match name {
                #(#objects)*
                _ => Err(#spice::serde::unknown(json)),
            }
        },
    };
    let decode = quote! {
        match #spice::serde::shape::<H>(json)? {
            #spice::serde::Shape::Name(name) => match name {
                #(#names)*
                _ => Err(#spice::serde::unknown(json)),
            },
            #spice::serde::Shape::Object(map) => { #object }
            #spice::serde::Shape::Array(items) => #array_decode,
        }
    };
    let encode = quote! {
        match self {
            #(#encodes)*
        }
    };

    Ok((decode, encode))
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
        let mut attrs = vec![];

        if field.key != field.rescript_name {
            attrs.push(format!("@spice.key({:?})", field.key));
        }

        if let Some(default) = &field.attr.default {
            attrs.push(format!("@spice.default({default})"));
        }

        let attrs = attrs.join(" ");
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

        if inline {
            let attrs = if attrs.is_empty() { attrs } else { format!("{attrs} ") };

            quote!(format!("{}{}: {}{}", #attrs, #name, #codec, #ty))
        } else {
            let docs = &field.docs;
            let attrs = if attrs.is_empty() {
                attrs
            } else {
                format!("  {attrs}\n")
            };

            quote!(format!("{}{}  {}: {}{},\n", #docs, #attrs, #name, #codec, #ty))
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
                (None, Some(rescript)) => Some(rust_default(rescript, &field.ty, field.ident.span())?),
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
    let container = ContainerAttr::parse(&input.attrs)?;
    let krate = container
        .krate
        .clone()
        .unwrap_or_else(|| syn::parse_quote!(::rescript_rs));
    let gen = Gen { krate };
    let spice = gen.spice();

    check_tuples(&input.data)?;

    if !input.generics.params.is_empty() {
        return Err(Error::new(input.generics.span(), "Spice types cannot be generic yet"));
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
    let (body, decode, encode) = match &input.data {
        Data::Struct(_) if container.serde => {
            return Err(Error::new(
                input.span(),
                "serde is for enums; a struct is a record either way",
            ))
        }
        Data::Struct(data) => derive_struct(&gen, &container, data, ident)?,
        Data::Enum(data) => derive_enum(&gen, &container, data)?,
        Data::Union(_) => return Err(Error::new(input.span(), "unions are not supported")),
    };

    let mut header = docs(&input.attrs, "");

    header.push_str(
        match (container.serde, container.only) {
            (true, Some(only)) => format!("@spice.serde {only}"),
            (true, None) => "@spice.serde".into(),
            (false, only) => only.unwrap_or("@spice").into(),
        }
        .as_str(),
    );

    if let Some(tag) = &container.tag {
        header.push_str(&format!(" @tag({tag:?})"));
    }

    if container.unboxed {
        header.push_str(" @unboxed");
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

    header.push_str(&format!("\ntype {rec}{name} ="));

    let declare = container.module.as_ref().map(|module| {
        quote! {
            #[automatically_derived]
            impl #spice::Declare for #ident {
                fn module() -> &'static str {
                    #module
                }

                fn declaration() -> ::std::string::String {
                    format!("{}{}", #header, #body)
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
        }

        #declare
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
            Type::Path(path) => path.path.segments.iter().try_for_each(|segment| {
                match &segment.arguments {
                    PathArguments::AngleBracketed(args) => {
                        args.args.iter().try_for_each(|arg| match arg {
                            GenericArgument::Type(ty) => check(ty),
                            _ => Ok(()),
                        })
                    }
                    _ => Ok(()),
                }
            }),
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
            if container.unboxed {
                return Err(Error::new(named.span(), "unboxed records are not supported"));
            }

            let fields = parse_fields(named, container.camel_case)?;
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
    alias: Option<AliasValue>,
    fields: Option<Vec<ParsedField>>,
}

fn alias_json(spice: &TokenStream, alias: &AliasValue) -> TokenStream {
    match alias {
        AliasValue::String(s) => quote!(#spice::Value::String(::std::string::String::from(#s))),
        AliasValue::Number(n) => quote!(H::number(#n)),
    }
}

fn alias_rescript(alias: &AliasValue) -> String {
    match alias {
        AliasValue::String(s) => format!("{s:?}"),
        AliasValue::Number(n) if n.fract() == 0.0 => format!("{}", *n as i64),
        AliasValue::Number(n) => format!("{n}"),
    }
}

fn derive_enum(gen: &Gen, container: &ContainerAttr, data: &syn::DataEnum) -> Result<Derived> {
    let spice = gen.spice();
    let variants = data
        .variants
        .iter()
        .map(|variant| {
            let VariantAttr { name, alias } = VariantAttr::parse(&variant.attrs)?;
            let fields = match &variant.fields {
                Fields::Named(_) if container.poly => {
                    return Err(Error::new(
                        variant.span(),
                        "a polymorphic variant constructor can't have named fields",
                    ))
                }
                Fields::Named(named) => Some(parse_fields(named, container.camel_case)?),
                _ => None,
            };

            Ok(ParsedVariant {
                variant,
                name: name.unwrap_or_else(|| variant.ident.unraw().to_string()),
                alias,
                fields,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let aliased = variants.iter().filter(|v| v.alias.is_some()).count();

    if container.serde {
        validate_serde(container, &variants)?;
    }

    if aliased > 0 && !container.serde && aliased != variants.len() {
        return Err(Error::new(
            data.variants.span(),
            "Partial @spice.as usage is not allowed",
        ));
    }

    if aliased > 0
        && !container.serde
        && variants
            .iter()
            .any(|v| !matches!(v.variant.fields, Fields::Unit))
    {
        return Err(Error::new(
            data.variants.span(),
            "@spice.as is only supported on constructors without payload",
        ));
    }

    let mut lines = vec![];

    for v in &variants {
        let constructor = format!("{}{}", if container.poly { "#" } else { "" }, v.name);
        let docs = docs(&v.variant.attrs, "  ");
        let alias = v
            .alias
            .as_ref()
            .map(|a| format!("@spice.as({}) ", alias_rescript(a)))
            .unwrap_or_default();

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

    if container.unboxed {
        return derive_unboxed_enum(gen, &variants, body);
    }

    if aliased > 0 && !container.serde {
        return Ok(derive_alias_enum(gen, &variants, body));
    }

    let kind = if container.poly {
        quote!(#spice::variant::Kind::Poly)
    } else {
        quote!(#spice::variant::Kind::Variant)
    };
    let mut decode_cases = vec![];
    let mut encode_cases = vec![];

    for v in &variants {
        let constructor = &v.variant.ident;
        let tag = &v.name;

        match &v.variant.fields {
            Fields::Unit => {
                decode_cases.push(quote! {
                    ::std::option::Option::Some(#tag) => {
                        #spice::variant::arity_of(#kind, &items, 0, json)?;

                        Ok(Self::#constructor)
                    }
                });
                encode_cases.push(quote! {
                    Self::#constructor => #spice::Value::Array(vec![#spice::Value::String(::std::string::String::from(#tag))]),
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

                decode_cases.push(quote! {
                    ::std::option::Option::Some(#tag) => {
                        #spice::variant::arity_of(#kind, &items, #count, json)?;

                        let (#(#vars,)*) = (#(#decodes,)*);

                        Ok(Self::#constructor(#(#finishes),*))
                    }
                });
                encode_cases.push(quote! {
                    Self::#constructor(#(#vars),*) => #spice::Value::Array(vec![
                        #spice::Value::String(::std::string::String::from(#tag)),
                        #(#spice::Spice::encode::<H>(#vars)),*
                    ]),
                });
            }
            Fields::Named(_) => {
                let fields = v.fields.as_ref().expect("named");
                let decode_fields = gen.decode_fields(fields, quote!(payload))?;
                let idents: Vec<&Ident> = fields.iter().map(|f| &f.ident).collect();
                let vars: Vec<Ident> = fields.iter().map(|f| field_var(&f.ident)).collect();
                let object = gen.encode_fields(fields, |field| quote!(#field));

                decode_cases.push(quote! {
                    ::std::option::Option::Some(#tag) => {
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
                encode_cases.push(quote! {
                    Self::#constructor { #(#idents),* } => #spice::Value::Array(vec![
                        #spice::Value::String(::std::string::String::from(#tag)),
                        #object,
                    ]),
                });
            }
        }
    }

    let array_decode = quote! {
        match #spice::variant::tag::<H>(&items) {
            #(#decode_cases)*
            _ => Err(#spice::variant::unknown_of(#kind, &items)),
        }
    };

    if container.serde {
        let (decode, encode) = derive_serde(gen, container, &variants, array_decode)?;

        return Ok((body, decode, encode));
    }

    let decode = quote! {
        let items = #spice::variant::items_of::<H>(#kind, json)?;

        #array_decode
    };
    let encode = quote! {
        match self {
            #(#encode_cases)*
        }
    };

    Ok((body, decode, encode))
}

fn derive_alias_enum(gen: &Gen, variants: &[ParsedVariant], body: TokenStream) -> Derived {
    let spice = gen.spice();
    let strings = variants.iter().filter_map(|v| match &v.alias {
        Some(AliasValue::String(s)) => {
            let constructor = &v.variant.ident;

            Some(quote!(#s => Ok(Self::#constructor),))
        }
        _ => None,
    });
    let numbers = variants.iter().filter_map(|v| match &v.alias {
        Some(AliasValue::Number(n)) => {
            let constructor = &v.variant.ident;

            Some(quote!(n if n == #n => Ok(Self::#constructor),))
        }
        _ => None,
    });
    let encodes = variants.iter().map(|v| {
        let constructor = &v.variant.ident;
        let json = alias_json(&spice, v.alias.as_ref().expect("aliased"));

        quote!(Self::#constructor => #json,)
    });

    let decode = quote! {
        match #spice::variant::alias::<H>(json)? {
            #spice::variant::Alias::String(s) => match s {
                #(#strings)*
                _ => Err(#spice::variant::not_matched(json)),
            },
            #spice::variant::Alias::Number(n) => match n {
                #(#numbers)*
                _ => Err(#spice::variant::not_matched(json)),
            },
        }
    };
    let encode = quote! {
        match self {
            #(#encodes)*
        }
    };

    (body, decode, encode)
}

fn derive_unboxed_enum(gen: &Gen, variants: &[ParsedVariant], body: TokenStream) -> Result<Derived> {
    let spice = gen.spice();

    let [variant] = variants else {
        return Err(Error::new(Span::call_site(), "an unboxed variant has one constructor"));
    };

    let Fields::Unnamed(unnamed) = &variant.variant.fields else {
        return Err(Error::new(
            variant.variant.span(),
            "an unboxed constructor has one unnamed payload",
        ));
    };

    if unnamed.unnamed.len() != 1 {
        return Err(Error::new(unnamed.span(), "Expected exactly one type argument"));
    }

    let ty = &unnamed.unnamed[0].ty;
    let constructor = &variant.variant.ident;
    let decode = quote!(<#ty as #spice::Spice>::decode::<H>(json).map(Self::#constructor));
    let encode = quote! {
        match self {
            Self::#constructor(v) => #spice::Spice::encode::<H>(v),
        }
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
                let refs: Vec<_> = fields.iter().map(|(i, t, w)| (i.clone(), *t, w.as_ref())).collect();
                let names: Vec<TokenStream> = fields.iter().map(|(i, _, _)| i.clone()).collect();

                Ok(record_samples(gen, &refs, |values| {
                    quote!(Self { #(#names: #values),* })
                }))
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
                        let samples = record_samples(gen, &fields, |values| {
                            quote!(Self::#constructor(#(#values),*))
                        });

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
                        let samples = record_samples(gen, &fields, |values| {
                            quote!(Self::#constructor { #(#names: #values),* })
                        });

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
