//! `#[rescript_rs::wasm]`: a Rust function, or a type's constructor and
//! methods, called from ReScript through wasm-bindgen with their own types
//! on both sides.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Attribute, Expr, ExprLit, FnArg, GenericArgument, Ident, ImplItem, ImplItemFn, Item, ItemFn,
    ItemImpl, Lit, LitStr, Meta, Pat, PathArguments, Result, ReturnType, Signature, Token, Type,
    Visibility,
};

use crate::spice::option_inner;

#[derive(Default)]
struct Args {
    codec: bool,
    name: Option<LitStr>,
    js_name: Option<LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut args = Args::default();

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            if key == "codec" {
                args.codec = true;
                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
                }
                continue;
            }
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

fn docs(attrs: &[Attribute]) -> Vec<String> {
    attrs
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

/// `f64`, `i32`, `u32` and `bool` cross as wasm-bindgen's own numbers and
/// booleans, with no `JsValue` and no serde. So does an `Option` of one:
/// its `None` is `undefined` in both directions, as ReScript's is.
fn native(ty: &Type) -> bool {
    scalar(ty) || option_inner(ty).is_some_and(scalar)
}

/// What a function answers as wasm-bindgen passes it: a native value, or
/// `()`, which JavaScript gets as `undefined`, ReScript's `unit`.
fn native_output(ty: &Type) -> bool {
    native(ty) || matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn scalar(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };

    path.qself.is_none()
        && ["f64", "i32", "u32", "bool"]
            .iter()
            .any(|name| path.path.is_ident(name))
}

fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);

    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }

    out
}

/// Whether `ty` is the type the impl block is for.
fn is_self(ty: &Type, owner: &Ident) -> bool {
    matches!(ty, Type::Path(path) if path.qself.is_none() && (path.path.is_ident("Self") || path.path.is_ident(owner)))
}

/// One callable: its description for `Module::function`/`Module::class`
/// and the body of its wasm-bindgen export.
struct Bound {
    /// `rescript_rs::spice::Function { … }`.
    describe: TokenStream,
    /// The export's parameters after the receiver.
    params: Vec<TokenStream>,
    returns: TokenStream,
    body: TokenStream,
    /// An `async fn`, whose export answers a `Promise`.
    asynchronous: bool,
}

fn bound(
    signature: &Signature,
    attrs: &[Attribute],
    codec: bool,
    name: String,
    js_name: String,
    call: impl FnOnce(Vec<TokenStream>) -> TokenStream,
    object: Option<TokenStream>,
) -> Result<Bound> {
    let asynchronous = signature.asyncness.is_some();

    if !signature.generics.params.is_empty() {
        syn_err_spanned!(&signature.generics; "a function called from ReScript cannot be generic");
    }

    let from_js = if codec {
        quote!(::rescript_rs::wasm::from_js)
    } else {
        quote!(::rescript_rs::wasm::from_serde)
    };
    let to_js = if codec {
        quote!(::rescript_rs::wasm::codec_result)
    } else {
        quote!(::rescript_rs::wasm::to_serde)
    };
    let js_value = quote!(::rescript_rs::wasm::JsValue);
    let mut params = vec![];
    let mut decode = vec![];
    let mut passed = vec![];
    let mut boundary = vec![];

    for input in &signature.inputs {
        let FnArg::Typed(input) = input else {
            continue;
        };
        if !matches!(&*input.pat, Pat::Ident(_)) {
            syn_err_spanned!(&input.pat; "name the parameter");
        }

        let (owned, borrowed) = parameter(&input.ty)?;
        let position = params.len();
        let arg = format_ident!("arg{}", position, span = input.pat.span());

        let native = !codec && native(&owned);

        if native {
            params.push(quote!(#arg: #owned));
        } else {
            params.push(quote!(#arg: #js_value));
            decode.push(quote!(let #arg = #from_js::<#owned>(&#arg, #position)?;));
        }
        passed.push(if borrowed {
            quote!(&#arg)
        } else {
            quote!(#arg)
        });

        let identity = if codec {
            quote!(false)
        } else if native {
            quote!(true)
        } else {
            quote!(<#owned as ::rescript_rs::spice::Wasm>::input_identity())
        };

        boundary.push(quote! {
            ::rescript_rs::spice::BoundaryType {
                ty: <#owned as ::rescript_rs::spice::Spice>::rescript_type(),
                identity: #identity,
            }
        });
    }

    let unit: Type = syn::parse_quote!(());
    let returned = match &signature.output {
        ReturnType::Default => &unit,
        ReturnType::Type(_, ty) => &**ty,
    };
    let (ok, fallible) = match fallible(returned) {
        Some(ok) => (ok, true),
        None => (returned, false),
    };
    let (returns, respond, output) = match object {
        // The class declares the object's type, `t`.
        Some(wrapper) => (
            wrapper.clone(),
            quote!(::core::result::Result::Ok(#wrapper(value))),
            quote! {
                ::rescript_rs::spice::BoundaryType { ty: "t".into(), identity: true }
            },
        ),
        None => {
            let native = !codec && native_output(ok);
            let identity = if codec {
                quote!(false)
            } else if native {
                quote!(true)
            } else {
                quote!(<#ok as ::rescript_rs::spice::Wasm>::output_identity())
            };
            let boundary = quote! {
                ::rescript_rs::spice::BoundaryType {
                    ty: <#ok as ::rescript_rs::spice::Spice>::rescript_type(),
                    identity: #identity,
                }
            };

            if native {
                (
                    quote!(#ok),
                    quote!(::core::result::Result::Ok(value)),
                    boundary,
                )
            } else {
                (js_value.clone(), quote!(#to_js(&value)), boundary)
            }
        }
    };
    let called = match asynchronous {
        true => {
            let called = call(passed);

            quote!(#called.await)
        }
        false => call(passed),
    };
    let body = if fallible {
        quote! {
            #(#decode)*

            match #called {
                ::core::result::Result::Ok(value) => #respond,
                ::core::result::Result::Err(error) => {
                    ::core::result::Result::Err(::rescript_rs::wasm::error(&error))
                }
            }
        }
    } else {
        quote! {
            #(#decode)*

            let value = #called;
            #respond
        }
    };
    let docs = docs(attrs);
    let describe = quote! {
        ::rescript_rs::spice::Function {
            name: #name,
            js_name: #js_name,
            docs: &[#(#docs),*],
            args: ::std::vec![#(#boundary),*],
            output: #output,
            asynchronous: #asynchronous,
        }
    };

    Ok(Bound {
        describe,
        params,
        returns: quote!(::core::result::Result<#returns, #js_value>),
        body,
        asynchronous,
    })
}

pub(crate) fn attribute(args: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let args: Args = syn::parse2(args)?;

    match syn::parse2::<Item>(item)? {
        Item::Fn(function) => function_attribute(args, function),
        Item::Impl(block) => class_attribute(args, block),
        other => {
            syn_err_spanned!(other; "#[rescript_rs::wasm] goes on a function or an impl block")
        }
    }
}

fn function_attribute(args: Args, function: ItemFn) -> Result<TokenStream> {
    let signature = &function.sig;

    if let Some(FnArg::Receiver(receiver)) = signature.inputs.first() {
        syn_err_spanned!(receiver; "a method is bound by #[rescript_rs::wasm] on its impl block");
    }

    let ident = &signature.ident;
    let name = args
        .name
        .map_or_else(|| camel_case(&ident.to_string()), |name| name.value());
    let js_name = args
        .js_name
        .map_or_else(|| name.clone(), |name| name.value());
    let Bound {
        describe,
        params,
        returns,
        body,
        asynchronous,
    } = bound(
        signature,
        &function.attrs,
        args.codec,
        name,
        js_name.clone(),
        |passed| quote!(#ident(#(#passed),*)),
        None,
    )?;
    let vis = &function.vis;
    // wasm-bindgen turns an async export's future into a `Promise` with
    // `wasm_bindgen_futures`, which a crate depending only on rescript-rs
    // reaches through it.
    let (asyncness, futures) = match asynchronous {
        true => (
            quote!(async),
            quote!(wasm_bindgen_futures = ::rescript_rs::wasm::wasm_bindgen_futures,),
        ),
        false => (quote!(), quote!()),
    };
    let describe_fn = format_ident!("{}_rescript", ident);
    let shim = format_ident!("{}_wasm", ident);

    Ok(quote! {
        #function

        /// How ReScript calls the function: add it to a
        /// `rescript_rs::spice::Module` with `function`.
        #[doc(hidden)]
        #vis fn #describe_fn() -> ::rescript_rs::spice::Function {
            #describe
        }

        ::rescript_rs::wasm_items! {
            #[::rescript_rs::wasm::wasm_bindgen::prelude::wasm_bindgen(
                js_name = #js_name,
                #futures
                wasm_bindgen = ::rescript_rs::wasm::wasm_bindgen
            )]
            #[doc(hidden)]
            #vis #asyncness fn #shim(#(#params),*) -> #returns {
                #body
            }
        }
    })
}

/// A bound method of the impl block: its wrapper method and description.
struct Member {
    describe: TokenStream,
    export: TokenStream,
}

fn class_attribute(args: Args, block: ItemImpl) -> Result<TokenStream> {
    if let Some((_, path, _)) = &block.trait_ {
        syn_err_spanned!(path; "#[rescript_rs::wasm] binds an inherent impl block, not a trait's");
    }
    if !block.generics.params.is_empty() {
        syn_err_spanned!(&block.generics; "a class called from ReScript cannot be generic");
    }

    let Type::Path(self_ty) = &*block.self_ty else {
        syn_err_spanned!(&block.self_ty; "#[rescript_rs::wasm] binds an impl block of a named type");
    };
    let Some(owner) = self_ty.path.get_ident().cloned() else {
        syn_err_spanned!(self_ty; "name the type without a path or type arguments");
    };
    let name = args
        .name
        .map_or_else(|| owner.to_string(), |name| name.value());
    let js_name = args
        .js_name
        .map_or_else(|| name.clone(), |name| name.value());
    let wrapper = format_ident!("{}Wasm", owner);
    let exports = format_ident!("__rescript_rs_{}", snake_case(&owner.to_string()));
    let mut constructor: Option<Member> = None;
    let mut methods = vec![];

    for item in &block.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        if !matches!(method.vis, Visibility::Public(_)) {
            continue;
        }

        let member = member(&owner, &wrapper, method, args.codec, &js_name)?;

        if method.sig.receiver().is_some() {
            methods.push(member);
        } else if constructor.is_some() {
            syn_err_spanned!(&method.sig; "a class has one constructor: make the other function private or free");
        } else {
            constructor = Some(member);
        }
    }

    let Some(constructor) = constructor else {
        syn_err_spanned!(&block.self_ty; "a class needs a constructor: a pub function returning Self");
    };
    let docs = docs(&block.attrs);
    let describe_fn = format_ident!("{}_rescript", snake_case(&owner.to_string()));
    let constructor_describe = &constructor.describe;
    let constructor_export = &constructor.export;
    let method_describes = methods.iter().map(|method| &method.describe);
    let method_exports = methods.iter().map(|method| &method.export);

    Ok(quote! {
        #block

        /// How ReScript constructs the class and calls its methods: add it
        /// to a `rescript_rs::spice::Module` with `class`.
        #[doc(hidden)]
        pub fn #describe_fn() -> ::rescript_rs::spice::Class {
            ::rescript_rs::spice::Class {
                name: #name,
                js_name: #js_name,
                docs: &[#(#docs),*],
                constructor: #constructor_describe,
                methods: ::std::vec![#(#method_describes),*],
            }
        }

        // wasm-bindgen's exports name the `wasm_bindgen` crate by a relative
        // path, which a crate without that dependency lacks. The struct sits
        // in a module that imports the name. Its methods import it in a
        // block, where a path in a signature (`super::Size`) still means
        // what it does in the impl block.
        ::rescript_rs::wasm_items! {
            #[doc(hidden)]
            mod #exports {
                use ::rescript_rs::wasm::wasm_bindgen;

                /// The JavaScript class: re-export it from the crate that
                /// builds the wasm module so the linker keeps it.
                #[wasm_bindgen::prelude::wasm_bindgen(
                    js_name = #js_name,
                    wasm_bindgen = ::rescript_rs::wasm::wasm_bindgen
                )]
                pub struct #wrapper(pub(super) super::#owner);
            }

            #[doc(hidden)]
            pub use #exports::#wrapper;

            const _: () = {
                use ::rescript_rs::wasm::wasm_bindgen;

                #[wasm_bindgen::prelude::wasm_bindgen(
                    js_class = #js_name,
                    wasm_bindgen = ::rescript_rs::wasm::wasm_bindgen
                )]
                impl #wrapper {
                    #constructor_export
                    #(#method_exports)*
                }
            };
        }
    })
}

fn member(
    owner: &Ident,
    wrapper: &Ident,
    method: &ImplItemFn,
    codec: bool,
    class: &str,
) -> Result<Member> {
    let signature = &method.sig;
    let ident = &signature.ident;

    if signature.asyncness.is_some() {
        syn_err_spanned!(signature; "an async method cannot be called from ReScript yet: make it a free async function");
    }

    let returned = match &signature.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(fallible(ty).unwrap_or(ty)),
    };
    let returns_self = returned.is_some_and(|ty| is_self(ty, owner));

    let Some(receiver) = signature.receiver() else {
        if !returns_self {
            syn_err_spanned!(signature; "a function without self is the constructor and returns Self: make it a free #[rescript_rs::wasm] function");
        }

        let Bound {
            describe,
            params,
            returns,
            body,
            ..
        } = bound(
            signature,
            &method.attrs,
            codec,
            "make".into(),
            class.into(),
            |passed| quote!(#owner::#ident(#(#passed),*)),
            Some(quote!(#wrapper)),
        )?;

        return Ok(Member {
            describe,
            export: quote! {
                #[wasm_bindgen(constructor)]
                pub fn #ident(#(#params),*) -> #returns {
                    #body
                }
            },
        });
    };

    if receiver.reference.is_none() || receiver.colon_token.is_some() {
        syn_err_spanned!(receiver; "a method called from ReScript takes &self or &mut self");
    }
    if returns_self {
        syn_err_spanned!(signature; "only the constructor returns Self");
    }

    let name = camel_case(&ident.to_string());

    // `make` constructs the object in ReScript and `free` releases it. A
    // method named `free` would also replace the JavaScript class's own.
    if matches!(name.as_str(), "make" | "free") {
        syn_err_spanned!(ident; "a class already has `{}`: name the method something else", name);
    }

    let Bound {
        describe,
        params,
        returns,
        body,
        ..
    } = bound(
        signature,
        &method.attrs,
        codec,
        name.clone(),
        name.clone(),
        |passed| quote!(self.0.#ident(#(#passed),*)),
        None,
    )?;
    let receiver = if receiver.mutability.is_some() {
        quote!(&mut self)
    } else {
        quote!(&self)
    };

    Ok(Member {
        describe,
        export: quote! {
            #[wasm_bindgen(js_name = #name)]
            pub fn #ident(#receiver, #(#params),*) -> #returns {
                #body
            }
        },
    })
}
