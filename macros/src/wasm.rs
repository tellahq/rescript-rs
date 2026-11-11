//! `#[rescript_rs::wasm]`: a Rust function called from ReScript through
//! wasm-bindgen with its own types on both sides.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Expr, ExprLit, FnArg, GenericArgument, ItemFn, Lit, LitStr, Meta, Pat, PathArguments, Result,
    ReturnType, Token, Type,
};

#[derive(Default)]
struct Args {
    name: Option<LitStr>,
    js_name: Option<LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut args = Args::default();

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: LitStr = input.parse()?;

            match key.to_string().as_str() {
                "name" => args.name = Some(value),
                "js_name" => args.js_name = Some(value),
                other => {
                    syn_err!(key.span(); "unknown argument `{}`: expected `name` or `js_name`", other)
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(args)
    }
}

fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;

    for c in name.chars() {
        if c == '_' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }

    out
}

/// A parameter as the shim reads it: the owned type its codec decodes, and
/// how the decoded value is handed to the function.
fn parameter(ty: &Type) -> Result<(Type, bool)> {
    let Type::Reference(reference) = ty else {
        return Ok((ty.clone(), false));
    };

    if reference.mutability.is_some() {
        syn_err_spanned!(ty; "a function called from ReScript cannot take `&mut`");
    }

    let owned = match &*reference.elem {
        Type::Path(path) if path.path.is_ident("str") => syn::parse_quote!(::std::string::String),
        Type::Slice(slice) => {
            let item = &slice.elem;

            syn::parse_quote!(::std::vec::Vec<#item>)
        }
        other => other.clone(),
    };

    Ok((owned, true))
}

/// `T` of a returned `Result<T, E>`.
fn fallible(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    let PathArguments::AngleBracketed(generics) = &last.arguments else {
        return None;
    };

    match generics.args.first()? {
        GenericArgument::Type(ok) if last.ident == "Result" && generics.args.len() == 2 => Some(ok),
        _ => None,
    }
}

fn docs(function: &ItemFn) -> Vec<String> {
    function
        .attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            Meta::NameValue(doc) if doc.path.is_ident("doc") => match &doc.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(text),
                    ..
                }) => Some(text.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(crate) fn attribute(args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let args: Args = syn::parse2(args)?;
    let function: ItemFn = syn::parse2(item)?;
    let signature = &function.sig;

    if signature.asyncness.is_some() {
        syn_err_spanned!(signature; "an async function cannot be called from ReScript yet");
    }
    if !signature.generics.params.is_empty() {
        syn_err_spanned!(&signature.generics; "a function called from ReScript cannot be generic");
    }

    let ident = &signature.ident;
    let name = args
        .name
        .map_or_else(|| camel_case(&ident.to_string()), |name| name.value());
    let js_name = args
        .js_name
        .map_or_else(|| name.clone(), |name| name.value());
    let mut names = vec![];
    let mut decoded = vec![];
    let mut passed = vec![];

    for (index, input) in signature.inputs.iter().enumerate() {
        let FnArg::Typed(input) = input else {
            syn_err_spanned!(input; "a method cannot be called from ReScript: use a free function");
        };
        if !matches!(&*input.pat, Pat::Ident(_)) {
            syn_err_spanned!(&input.pat; "name the parameter");
        }

        let (owned, borrowed) = parameter(&input.ty)?;
        let arg = format_ident!("arg{}", index, span = input.pat.span());

        passed.push(if borrowed {
            quote!(&#arg)
        } else {
            quote!(#arg)
        });
        names.push(arg);
        decoded.push(owned);
    }

    let unit: Type = syn::parse_quote!(());
    let returned = match &signature.output {
        ReturnType::Default => &unit,
        ReturnType::Type(_, ty) => &**ty,
    };
    let (output, respond) = match fallible(returned) {
        Some(ok) => (
            ok,
            quote! {
                match #ident(#(#passed),*) {
                    ::core::result::Result::Ok(value) => {
                        ::core::result::Result::Ok(::rescript_rs::wasm::to_js(&value))
                    }
                    ::core::result::Result::Err(error) => {
                        ::core::result::Result::Err(::rescript_rs::wasm::error(&error))
                    }
                }
            },
        ),
        None => (
            returned,
            quote!(::core::result::Result::Ok(::rescript_rs::wasm::to_js(&#ident(#(#passed),*)))),
        ),
    };
    let vis = &function.vis;
    let describe = format_ident!("{}_rescript", ident);
    let shim = format_ident!("{}_wasm", ident);
    let docs = docs(&function);
    let positions = 0..names.len();

    Ok(quote! {
        #function

        /// How ReScript calls the function: add it to a
        /// `rescript_rs::spice::Module` with `function`.
        #[doc(hidden)]
        #vis fn #describe() -> ::rescript_rs::spice::Function {
            ::rescript_rs::spice::Function {
                name: #name,
                js_name: #js_name,
                docs: &[#(#docs),*],
                args: ::std::vec![
                    #(<#decoded as ::rescript_rs::spice::Spice>::rescript_type()),*
                ],
                output: <#output as ::rescript_rs::spice::Spice>::rescript_type(),
            }
        }

        ::rescript_rs::wasm_shim! {
            #js_name
            #[doc(hidden)]
            #vis fn #shim(
                #(#names: ::rescript_rs::wasm::JsValue),*
            ) -> ::core::result::Result<::rescript_rs::wasm::JsValue, ::rescript_rs::wasm::JsValue> {
                #(let #names = ::rescript_rs::wasm::from_js::<#decoded>(&#names, #positions)?;)*

                #respond
            }
        }
    })
}
