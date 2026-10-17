//! Attribute macros mirroring Kani's, for running harnesses as tests.

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

/// Name of the function defined by `item` (the identifier after `fn`).
fn fn_name(item: &TokenStream) -> Ident {
    let mut saw_fn = false;
    for tt in item.clone() {
        if let TokenTree::Ident(id) = tt {
            if saw_fn {
                return id;
            }
            saw_fn = id.to_string() == "fn";
        }
    }
    panic!("#[kani::proof] must be applied to a function");
}

fn path(segments: &[&str]) -> TokenStream {
    let mut out = Vec::new();
    for s in segments {
        out.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
        out.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
        out.push(TokenTree::Ident(Ident::new(s, Span::call_site())));
    }
    out.into_iter().collect()
}

/// `#[kani::proof]`: wraps the harness in a `#[test]` that runs it many
/// times with random `kani::any()` values.
#[proc_macro_attribute]
pub fn proof(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = fn_name(&item);
    let mut body: Vec<TokenTree> = item.clone().into_iter().collect();
    body.push(TokenTree::Punct(Punct::new(';', Spacing::Alone)));
    body.extend(path(&["kani", "__run"]));
    let args: TokenStream = [
        TokenTree::Literal(Literal::string(&name.to_string())),
        TokenTree::Punct(Punct::new(',', Spacing::Alone)),
        TokenTree::Ident(name.clone()),
    ]
    .into_iter()
    .collect();
    body.push(TokenTree::Group(Group::new(Delimiter::Parenthesis, args)));
    body.push(TokenTree::Punct(Punct::new(';', Spacing::Alone)));

    let mut out: Vec<TokenTree> = Vec::new();
    out.push(TokenTree::Punct(Punct::new('#', Spacing::Alone)));
    out.push(TokenTree::Group(Group::new(
        Delimiter::Bracket,
        TokenStream::from(TokenTree::Ident(Ident::new("test", Span::call_site()))),
    )));
    out.push(TokenTree::Ident(Ident::new("fn", Span::call_site())));
    out.push(TokenTree::Ident(name));
    out.push(TokenTree::Group(Group::new(Delimiter::Parenthesis, TokenStream::new())));
    out.push(TokenTree::Group(Group::new(Delimiter::Brace, body.into_iter().collect())));
    out.into_iter().collect()
}

/// Kani-only tuning attributes: accepted and ignored.
#[proc_macro_attribute]
pub fn unwind(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Accepted and ignored.
#[proc_macro_attribute]
pub fn solver(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Accepted and ignored.
#[proc_macro_attribute]
pub fn stub(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
