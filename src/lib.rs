use std::collections::HashMap;

use proc_macro::TokenStream;

use proc_macro2::{Literal, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Data, Error, Ident, Lit, LitStr, Result, Token,
};

#[proc_macro_derive(UserData, attributes(userdata))]
pub fn user_data_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_user_data(&ast)
}

fn impl_user_data(ast: &syn::DeriveInput) -> TokenStream {
    let fields = match &ast.data {
        Data::Struct(s) => &s.fields,
        _ => panic!("`UserData` can only be derived on Structs."),
    };

    let struct_name = &ast.ident;
    // Get the struct-wide opts
    // also wow this code is awfully Rusty isn't it
    let structwide_opts = match ast
        .attrs
        .iter()
        .find(|a| a.path == syn::parse2(quote! {userdata}).unwrap())
        .map(|a| a.parse_args::<StructwideOpts>())
    {
        Some(Ok(opts)) => opts,
        Some(Err(oh_no)) => return oh_no.to_compile_error().into(),
        None => StructwideOpts::default(),
    };

    // Map the index of the field to its information
    let mut field_infos = HashMap::new();
    let fields_vec = fields.iter().collect::<Vec<_>>();
    if fields_vec.is_empty() {
        // no need to continue
        // return nothing
        return TokenStream::new();
    }
    for (idx, f) in fields_vec.iter().enumerate() {
        for attr in f.attrs.iter() {
            if attr.path.is_ident("userdata") {
                // this is our stop!
                let field_info = if attr.tokens.is_empty() {
                    FieldInfo::default()
                } else {
                    match attr.parse_args() {
                        Ok(v) => v,
                        Err(e) => return e.to_compile_error().into(),
                    }
                };
                field_infos.insert(idx, field_info);
            }
        }
    }

    // Now let's make the token streams!
    let mut index_toks = TokenStream2::new();
    let mut newindex_toks = TokenStream2::new();

    // Note to self:
    // Index calls with (reqd_key) and returns the value
    // NewIndex calls with (reqd_key, val) and assigns the value.

    for (idx, field) in fields_vec.iter().enumerate() {
        // this unwrap is OK because we made sure all these idxes exist already
        let field_info = field_infos.get(&idx).unwrap();

        // Get the name of the field
        // If it has no name, use its index in the struct
        let key_name_for_lua = match &field_info.rename {
            Some(rename_to) => Some(rename_to.clone()),
            None => {
                // We gotta figure it out ourselves
                match &field.ident {
                    Some(field_name) => Some(field_name.to_string()),
                    None => None,
                }
            }
        };

        if field_info.read {
            // Add the Index method match branch
            match &key_name_for_lua {
                Some(field_name) => {
                    // We're looking for string keys
                    let field_name_as_bytes = field_name.as_bytes();
                    let field_name_as_bytes = Literal::byte_string(field_name_as_bytes);

                    // this unwrap is OK cause we checked above if this is a string
                    let actual_field_name = field.ident.as_ref().unwrap();

                    index_toks.extend(quote! {
                        Value::String(key) if key.as_bytes() == #field_name_as_bytes => {
                            Ok(ToLua::to_lua(this.#actual_field_name.clone(), ctx)?)
                        },
                    });
                }
                None => {
                    // We're looking for numerical keys
                    index_toks.extend(quote! {
                        Value::Integer(key) if key == #idx + 1 as LuaInteger => {
                            Ok(ToLua::to_lua(this.#idx.clone(), ctx)?)
                        },
                    });
                }
            }
        }

        if field_info.write {
            // do NewIndex method
            match &key_name_for_lua {
                Some(field_name) => {
                    let field_name_as_bytes = field_name.as_bytes();
                    let field_name_as_bytes = Literal::byte_string(field_name_as_bytes);

                    let actual_field_name = field.ident.as_ref().unwrap();

                    newindex_toks.extend(quote! {
                        Value::String(key) if key.as_bytes() == #field_name_as_bytes => {
                            this.#actual_field_name = FromLua::from_lua(val, ctx)?;
                            Ok(())
                        },
                    });
                }
                None => {
                    newindex_toks.extend(quote! {
                        Value::Integer(key) if key == #idx + 1 as LuaInteger => {
                            this.#idx = FromLua::from_lua(val, ctx)?;
                            Ok(())
                        },
                    });
                }
            }
        }
    }

    // Make up the match statements
    let match_index_toks = quote! {
        match key {
            #index_toks
            _ => Err(Error::RuntimeError(format!("unknown key `{:?}`", key)))
        }
    };

    let match_newindex_toks = quote! {
        match key {
            #newindex_toks
            _ => Err(Error::RuntimeError(format!("unknown key `{:?}`", key)))
        }
    };

    let crate_root = structwide_opts.crate_root;
    let out = quote! {
        impl #crate_root::rlua::UserData for #struct_name {
            fn add_methods<'lua, M: #crate_root::rlua::UserDataMethods<'lua, Self>>(methods: &mut M) {
                use #crate_root::rlua::{ToLua, FromLua, MetaMethod, Error, Value, Integer as LuaInteger};

                methods.add_meta_method(MetaMethod::Index, |ctx, this, key: Value| -> Result<Value, Error> {
                    #match_index_toks
                });

                methods.add_meta_method_mut(MetaMethod::NewIndex, |ctx, this, (key, val): (Value, Value)| -> Result<(), Error> {
                    #match_newindex_toks
                });
            }
        }
    };
    // println!("{}", out);
    out.into()
}

#[derive(Debug)]
struct FieldInfo {
    read: bool,
    write: bool,
    rename: Option<String>,
}

impl Default for FieldInfo {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            rename: None,
        }
    }
}

impl Parse for FieldInfo {
    fn parse(input: ParseStream) -> Result<Self> {
        // we edit this over time
        let mut info = FieldInfo {
            read: false,
            write: false,
            rename: None,
        };

        let entries = Punctuated::<FieldEntry, Token![,]>::parse_separated_nonempty(input)?;
        for entry in entries {
            match entry {
                FieldEntry::Literal(i) => {
                    if i == "read" {
                        info.read = true;
                    } else {
                        return Err(Error::new(i.span(), "This identifier is not allowed here"));
                    }
                }
                FieldEntry::KeyValue(k, v) => {
                    if k == "rename" {
                        info.rename = Some(v.value());
                    } else {
                        return Err(Error::new(k.span(), "This identifier is not allowed here"));
                    }
                }
            }
        }
        Ok(info)
    }
}

/// `read` or `rename = "name"`
enum FieldEntry {
    Literal(Ident),
    KeyValue(Ident, LitStr),
}

impl Parse for FieldEntry {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = input.parse::<Ident>()?;
        Ok(if input.peek(Token![=]) {
            // Key-value entry
            // Consume the =
            input.parse::<Token![=]>()?;
            let literal_val: Lit = input.parse()?;
            let str_val = match literal_val {
                Lit::Str(s) => s,
                _ => {
                    return Err(Error::new(
                        literal_val.span(),
                        "Only str literals are allowed here",
                    ))
                }
            };
            FieldEntry::KeyValue(ident, str_val)
        } else {
            // Literal entry
            FieldEntry::Literal(ident)
        })
    }
}

/// Additional information that applies to the whole struct
struct StructwideOpts {
    crate_root: syn::Path,
}

impl Default for StructwideOpts {
    fn default() -> Self {
        Self {
            crate_root: syn::parse2(quote!(::rud_internal)).unwrap(),
        }
    }
}

impl Parse for StructwideOpts {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut crate_root = None;

        let entries = Punctuated::<FieldEntry, Token![,]>::parse_separated_nonempty(input)?;
        for entry in entries {
            match entry {
                FieldEntry::Literal(i) => {
                    return Err(Error::new(i.span(), "This identifier is not allowed here"))
                }
                FieldEntry::KeyValue(k, v) => {
                    if k == "crate" {
                        crate_root = Some(v.parse::<syn::Path>()?);
                    } else {
                        return Err(Error::new(k.span(), "This identifier is not allowed here"));
                    }
                }
            }
        }

        Ok(Self {
            crate_root: crate_root.unwrap_or_else(|| syn::parse2(quote!(::rud_internal)).unwrap()),
        })
    }
}
