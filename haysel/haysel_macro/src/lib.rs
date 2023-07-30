#![feature(proc_macro_diagnostic)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, spanned::Spanned, AngleBracketedGenericArguments, BinOp, Block, Data,
    DeriveInput, Expr, ExprBinary, ExprBlock, ExprLit, ExprPath, Fields, GenericArgument, Lit,
    Path, PathArguments, PathSegment, Stmt, Type, TypePath,
};

/// Horrifically patched together derive system for the Info debug trait.
///
/// this is less stable than a house of cards, and ONLY supports EXACTLY
/// the level of functionality needed for the structs in use in `tsdb2::repr`
#[proc_macro_derive(Info)]
pub fn info(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as DeriveInput);
    let fields = parse_fields(&input).into_iter().map(|field| {
        let name = field.0;
        let is_pointer = match &field.1 {
            Kind::Type(..) | Kind::Array(..) | Kind::ChunkedLinkedList(..) => false,
            Kind::Pointer(..) | Kind::PointerChunkedLinkedList(..) => true,
        };
        let (kind, path) = match field.1 {
            Kind::Type(path) | Kind::Pointer(path) => (
                quote! {
                    crate::tsdb2::repr::info::FieldKind::Single {
                        name: #name.into(),
                        typ: crate::tsdb2::repr::info::type_name::<#path>(),
                        size: std::mem::size_of::<#path>(),
                    }
                },
                path,
            ),
            Kind::Array(len, path) => {
                let len_name = expr2name(len.clone());
                (
                    quote! {
                        crate::tsdb2::repr::info::FieldKind::Array {
                            name: #name.into(),
                            elem_t: crate::tsdb2::repr::info::type_name::<#path>(),
                            elem_size: std::mem::size_of::<#path>(),
                            len_name: #len_name.into(),
                            len: #len,
                            total_size: std::mem::size_of::<[#path; #len]>(),
                        }
                    },
                    path,
                )
            }
            Kind::ChunkedLinkedList(len, path) | Kind::PointerChunkedLinkedList(len, path) => {
                let len_name = expr2name(len.clone());
                (
                    quote! {
                        crate::tsdb2::repr::info::FieldKind::ChunkedLinkedList {
                            name: #name.into(),
                            metadata_size: std::mem::size_of::<ChunkedLinkedList<0, ()>>(),
                            elem_t: crate::tsdb2::repr::info::type_name::<#path>(),
                            elem_size: std::mem::size_of::<#path>(),
                            chunk_len_name: #len_name.into(),
                            chunk_len: #len,
                            total_size: std::mem::size_of::<ChunkedLinkedList<#len, #path>>(),
                        }
                    },
                    path,
                )
            }
        };
        quote! {
            crate::tsdb2::repr::info::Field {
                is_pointer: #is_pointer,
                kind: #kind,
                info_impl: <#path as crate::tsdb2::repr::info::Info>::info2
            }
        }
    });
    let name = input.ident;

    // Build the output, possibly using quasi-quotation
    let expanded = quote! {
        impl crate::tsdb2::repr::info::Info for #name {
            fn info2() -> Option<Vec<crate::tsdb2::repr::info::Field>> {
                Some(vec![
                    #(#fields),*
                ])
            }
        }
    };

    // Hand the output tokens back to the compiler
    TokenStream::from(expanded)
}

#[derive(Debug)]
enum Kind {
    Type(Path),
    Pointer(Path),
    Array(Expr, Path),
    ChunkedLinkedList(Expr, Path),
    PointerChunkedLinkedList(Expr, Path),
}

fn expr2name(e: Expr) -> String {
    match e {
        Expr::Block(block) => {
            let ExprBlock {
                block: Block { stmts, .. },
                ..
            } = block;
            assert_eq!(stmts.len(), 1);
            let Stmt::Expr(
                Expr::Path(ExprPath {
                    path: Path { segments, .. },
                    ..
                }),
                ..,
            ) = stmts.first().unwrap()
            else {
                unimplemented!("e2n 2")
            };
            let PathSegment { ident, .. } = segments.last().unwrap();
            ident.to_string()
        }
        Expr::Lit(lit) => {
            let ExprLit {
                lit: Lit::Int(int), ..
            } = lit
            else {
                unreachable!("e2n non-int literal")
            };
            int.to_string()
        }
        Expr::Path(path) => {
            let ExprPath {
                path: Path { segments, .. },
                ..
            } = path;
            segments.last().unwrap().ident.to_string()
        }
        Expr::Binary(bin) => {
            let ExprBinary {
                left, op, right, ..
            } = bin;
            let Expr::Path(ExprPath {
                path: Path { segments, .. },
                ..
            }) = *left
            else {
                unimplemented!("e2n bin op left not path")
            };
            let BinOp::Sub(..) = op else {
                unimplemented!("e2n bin op non sub")
            };
            let Expr::Lit(ExprLit {
                lit: Lit::Int(int), ..
            }) = *right
            else {
                unimplemented!("e2n bin op right not lit")
            };
            segments.last().unwrap().ident.to_string() + "-" + int.to_string().as_str()
        }
        other => unimplemented!("e2n: other literal: {other:#?}"),
    }
}

fn parse_fields(input: &DeriveInput) -> Vec<(String, Kind)> {
    match input.data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    let fields_vals = fields.named.iter().map(|field| {

                        let kind = match field.ty {
                            Type::Path(ref path) => {
                                assert_eq!(
                                    path.path.segments.len(),
                                    1,
                                    "Info derive cannot operate on types speficied by path"
                                );
                                let PathSegment {
                                    ident, arguments, ..
                                } = &path.path.segments[0];
                                // behold the horrifying code
                                if ident.to_string() == "Ptr" {
                                    let PathArguments::AngleBracketed(
                                        AngleBracketedGenericArguments { args, .. },
                                    ) = arguments
                                    else {
                                        unimplemented!("struct path ptr PathArguments::AngleBracketed")
                                    };
                                    assert_eq!(args.len(), 1);
                                    let GenericArgument::Type(Type::Path(path)) =
                                        args.first().unwrap()
                                    else {
                                        unimplemented!("struct path ptr GenericArgument::Type")
                                    };
                                    Kind::Pointer(path.path.clone())
                                } else {
                                    Kind::Type(path.path.clone())
                                }
                            }
                            Type::Array(ref array) => {
                                let len = array.len.clone();
                                let Type::Path(ref typ) = *array.elem else {
                                    unimplemented!("struct array Type::Path")
                                };
                                Kind::Array(len, typ.path.clone())
                            }
                            ref other => {
                                other
                                    .span()
                                    .unwrap()
                                    .help("Unsupported type use here")
                                    .emit();
                                panic!("Info derive does not support this type ({other:?})")
                            }
                        };

                        let kind = match kind {
                            Kind::Type(ref path) | Kind::Pointer(ref path) => {
                                assert_eq!(
                                    path.segments.len(),
                                    1,
                                    "Info derive cannot operate on types speficied by path"
                                );
                                let PathSegment {
                                    ident, arguments, ..
                                } = &path.segments[0];
                                if ident.to_string() == "ChunkedLinkedList" {
                                    let PathArguments::AngleBracketed(
                                        AngleBracketedGenericArguments { args, .. },
                                    ) = arguments
                                    else {
                                        unimplemented!("match kind LinkedList PathArguments::AngleBracketed")
                                    };
                                    assert_eq!(args.len(), 2);
                                    let GenericArgument::Const(expr) =
                                        args.first().unwrap().clone()
                                    else {
                                        unimplemented!("match kind LinkedList GenericArgument::Const")
                                    };
                                    let GenericArgument::Type(Type::Path(TypePath {
                                        path, ..
                                    })) = args.last().unwrap().clone()
                                    else {
                                        unimplemented!("match kind LinkedList GenericArgument::Type")
                                    };
                                    match kind {
                                        Kind::Type(..) => Kind::ChunkedLinkedList(expr, path),
                                        Kind::Pointer(..) => {
                                            Kind::PointerChunkedLinkedList(expr, path)
                                        }
                                        _ => unreachable!("match kind LinkedList match kind"),
                                    }
                                } else {
                                    kind
                                }
                            }
                            other => other,
                        };
                        (field.ident.as_ref().unwrap().to_string(), kind)
                    }).collect::<Vec<_>>();
                    fields_vals
                }
                Fields::Unnamed(..) => {
                    input
                        .span()
                        .unwrap()
                        .help("Tuple struct declared here")
                        .emit();
                    panic!("Info derive does not support tuple structs")
                }
                Fields::Unit => {
                    input
                        .span()
                        .unwrap()
                        .help("Unit struct declared here")
                        .emit();
                    panic!("Info derive does not support unit structs")
                }
            }
        }
        Data::Enum(ref _data) => unimplemented!("enum"),
        Data::Union(ref _data) => unimplemented!("union"),
    }
}
