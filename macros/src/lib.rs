use syn;

#[proc_macro_attribute]
pub fn start_func_with(code: TokenStream, input: TokenStream) -> TokenStream {
    let mut item: syn::Item = syn::parse(input).unwrap();
    let fn_item = match &mut item {
        syn::Item::Fn(fn_item) => fn_item,
        _ => panic!("expected fn"),
    };
    fn_item.block.stmts.insert(0, syn::parse(code).unwrap());

    use quote::ToTokens;
    item.into_token_stream().into()
}
