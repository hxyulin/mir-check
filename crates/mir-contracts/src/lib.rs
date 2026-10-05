//! Contracts for static analysis. Attributes preserve function bodies without runtime checks.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use quote::{ToTokens, quote};

/// Requests proof that a function cannot panic under its verified preconditions.
#[proc_macro_attribute]
pub fn no_panic(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro::Span::call_site().into(),
            "no_panic takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    annotate("no_panic".to_owned(), item)
}

/// Declares a precondition. Syntax is checked; names and truth are not yet verified.
#[proc_macro_attribute]
pub fn requires(args: TokenStream, item: TokenStream) -> TokenStream {
    predicate("requires", args, item)
}

/// Declares a postcondition; `result` names the return value for future verification.
#[proc_macro_attribute]
pub fn ensures(args: TokenStream, item: TokenStream) -> TokenStream {
    predicate("ensures", args, item)
}

fn predicate(kind: &str, args: TokenStream, item: TokenStream) -> TokenStream {
    match syn::parse::<syn::Expr>(args) {
        Ok(expression) => annotate(format!("{kind}:{}", expression.to_token_stream()), item),
        Err(error) => error.to_compile_error().into(),
    }
}

fn annotate(payload: String, item: TokenStream) -> TokenStream {
    let item = match syn::parse::<syn::ItemFn>(item) {
        Ok(item) => item,
        Err(error) => return error.to_compile_error().into(),
    };
    let metadata = format!("<!-- mir-checker:v1:{payload} -->");
    quote!(#[doc = #metadata] #item).into()
}
