extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, parse_macro_input};

/// Implements the Runnable trait for a Reporter struct.
///
/// This macro automatically generates the implementation of the Runnable trait
/// for any struct that has the `broker` field and a `report` method.
///
/// # Example
///
/// ```rust,ignore
/// use pulse_proc_macros::reporter;
/// use crate::broker::Broker;
/// use crate::message::EndpointHistory;
/// use std::thread::JoinHandle;
///
/// #[reporter]
/// struct MyReporter {
///     name: String,
///     broker: Broker,
///     // other fields...
/// }
///
/// impl MyReporter {
///     async fn report(&self, report: EndpointHistory) -> Vec<JoinHandle<()>> {
///         // Your custom reporting logic here
///         vec![]
///     }
/// }
/// ```
///
/// # Errors
///
/// This macro will emit a compile error if:
/// - The annotated item is not a struct
/// - The struct doesn't have a `name` field of type String
/// - The struct doesn't have a `broker` field of type Broker
#[proc_macro_attribute]
pub fn reporter(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(item as DeriveInput);

    // Get the name of the struct
    let struct_name = &input.ident;

    // Verify that we're dealing with a struct
    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => &fields_named.named,
            _ => {
                return TokenStream::from(
                    Error::new_spanned(
                        struct_name,
                        "reporter attribute can only be applied to structs with named fields",
                    )
                    .to_compile_error(),
                );
            }
        },
        _ => {
            return TokenStream::from(
                Error::new_spanned(
                    struct_name,
                    "reporter attribute can only be applied to structs",
                )
                .to_compile_error(),
            );
        }
    };

    // Check if the struct has the required fields
    let mut has_name = false;
    let mut has_broker = false;
    let mut name_type = None;
    let mut broker_type = None;

    for field in fields {
        if let Some(ident) = &field.ident {
            if ident == "name" {
                has_name = true;
                name_type = Some(&field.ty);
            } else if ident == "broker" {
                has_broker = true;
                broker_type = Some(&field.ty);
            }
        }
    }

    if !has_name {
        return TokenStream::from(
            Error::new_spanned(struct_name, "reporter requires a 'name' field").to_compile_error(),
        );
    }

    if !has_broker {
        return TokenStream::from(
            Error::new_spanned(struct_name, "reporter requires a 'broker' field")
                .to_compile_error(),
        );
    }

    // Check field types
    if let Some(name_type) = name_type {
        let name_type_str = quote!(#name_type).to_string();
        if !name_type_str.contains("String") {
            return TokenStream::from(
                Error::new_spanned(struct_name, "reporter requires a 'name' field of type String")
                    .to_compile_error(),
            );
        }
    }

    if let Some(broker_type) = broker_type {
        let broker_type_str = quote!(#broker_type).to_string();
        if !broker_type_str.contains("Broker") {
            return TokenStream::from(
                Error::new_spanned(
                    struct_name,
                    "reporter requires a 'broker' field of type Broker",
                )
                .to_compile_error(),
            );
        }
    }

    // Generate the implementation of the Runnable trait
    let expanded = quote! {
        #input

        #[async_trait::async_trait]
        impl crate::runnable::Runnable for #struct_name {
            async fn run(&mut self) {
                loop {
                    match self.broker.receive_report().await {
                        Ok(report) => {
                            <Self as crate::agent::reporter::executor::Executor>::report(self, report).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!("[reporter] receiver lagged behind, skipped {} messages", skipped);
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!("[reporter] receive report error: {}", e);
                            break;
                        }
                    }
                    if self.broker.is_shutdown() {
                        break;
                    }
                }
            }

            fn name(&self) -> &str {
                &self.name
            }
        }

        const _: fn() = || {
            fn assert_executor<T: crate::agent::reporter::executor::Executor>() {}
            assert_executor::<#struct_name>();
        };
    };

    // Convert back to a token stream and return it
    TokenStream::from(expanded)
}
