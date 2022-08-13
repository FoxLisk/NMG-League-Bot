use proc_macro::{ TokenStream};
use quote::quote;
use syn::{Data, DeriveInput, Fields, Variant};

#[proc_macro_derive(DieselEnum)]
/// Given an enum `E` with only unit variants, this will derive
/// `TryFrom<String>` for `E`, and `From<E>` for `String`.
///
/// Then you can add this to your diesel types. e.g. given
///
/// ```
/// #[derive(DieselEnum)]
/// enum State {
///     Variant,
/// }
/// ```
///
/// you can have a Queryable struct like
///
/// ```
/// #[derive(Queryable)]
/// struct HasState {
///     #[diesel(deserialize=String)]
///     state: State,
/// }
/// ```

pub fn diesel_enum_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    let mut into_string = derive_into_string(&ast);

    let tryfrom_string = derive_tryfrom_string(&ast);

    into_string.extend(tryfrom_string);

    into_string
}

fn derive_into_string(ast: &DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let enum_data = match &ast.data {
        Data::Enum(ref enum_data) => enum_data,
        _ => {panic!("Can only derive this on enums") }
    };
    let mut fields = vec![];
    for v in enum_data.variants.iter() {
        match v.fields {
            Fields::Unit => {
                fields.push(v);
            }
            _ => {panic!("Can only derive for unit enums");}
        }
    }
    let field_tokens = fields.iter().map(|f| {
        let ident_str = format!("{}", f.ident);
        let ident = &f.ident;
        quote!(
            #name :: #ident => #ident_str
        )
    });
    let gen = quote! {
        impl From<#name> for String {
            fn from(e: #name) -> String {
                match e {
                    #(#field_tokens ,)*
                }.to_string()
            }
        }
    };
    gen.into()
}


fn derive_tryfrom_string(ast: &DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let enum_data = match &ast.data {
        Data::Enum(ref enum_data) => enum_data,
        _ => {panic!("Can only derive this on enums") }
    };
    let mut fields = vec![];
    for v in enum_data.variants.iter() {
        match v.fields {
            Fields::Unit => {
                fields.push(v);
            }
            _ => {panic!("Can only derive for unit enums");}
        }
    }
    let field_tokens = fields.iter().map(|f| {
        let ident_str = format!("{}", f.ident);
        let ident = &f.ident;
        quote!(
            #ident_str => Self :: #ident
        )
    });
    let err = format!("Not a valid {}", name);
    let gen = quote! {
        impl TryFrom<String> for #name {
            type Error = &'static str;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Ok(match value.as_str() {
                    #( #field_tokens ,)*
                    _ => {
                        return Err(#err);
                    }
                })
            }
        }
    };
    gen.into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
