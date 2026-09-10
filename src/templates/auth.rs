//! Templates for the auth pages (setup wizard + login).
//!
//! Both pages render through [`crate::templates::layout::base`] with no
//! active nav tab and no CSRF meta tag — they are public, pre-auth screens.
//! The form controls reuse the global `field` class from the base layout, so
//! the auth screens add no styling of their own.

use maud::{html, Markup};

use crate::templates::layout::base;

/// First-run setup wizard form.
///
/// Renders a centered card with a heading, subtitle, and a `POST /setup`
/// form containing `password` and `confirm` fields. No CSRF token is
/// rendered because setup runs before any session exists.
pub fn setup_page() -> Markup {
    base(
        "rusno setup",
        "",
        None,
        html! {
            div class="card" style="max-width: 28rem; margin: 3rem auto;" {
                h1 style="font-size: 1.5rem; margin-bottom: 0.25rem;" {
                    "Welcome to rusno"
                }
                p style="color: #9aa0b5; margin-bottom: 1.25rem;" {
                    "Set your admin password to get started."
                }
                form method="post" action="/setup" {
                    label for="password" class="field" {
                        "Password"
                        input type="password" id="password" name="password"
                            required autocomplete="new-password";
                    }
                    label for="confirm" class="field" {
                        "Confirm password"
                        input type="password" id="confirm" name="confirm"
                            required autocomplete="new-password";
                    }
                    button type="submit" class="btn btn-primary" style="width:100%;" {
                        "Set password"
                    }
                }
            }
        },
    )
}

/// Login form, optionally with an error alert.
///
/// * `error` — when `Some(msg)`, a red alert box is rendered above the form
///   (e.g. `"Invalid password"`). When `None`, no alert is shown.
///
/// Like [`setup_page`], no CSRF token is rendered and the form posts to
/// `/login`.
pub fn login_page(error: Option<&str>) -> Markup {
    base(
        "rusno login",
        "",
        None,
        html! {
            div class="card" style="max-width: 28rem; margin: 3rem auto;" {
                h1 style="font-size: 1.5rem; margin-bottom: 0.25rem;" { "rusno" }
                p style="color: #9aa0b5; margin-bottom: 1.25rem;" {
                    "Sign in to continue."
                }
                @if let Some(msg) = error {
                    div
                        style="background: rgba(231, 76, 60, 0.15); color: #e74c3c; \
                            border: 1px solid rgba(231, 76, 60, 0.4); border-radius: 6px; \
                            padding: 0.65rem 0.85rem; margin-bottom: 1rem; font-size: 0.9rem;"
                    {
                        (msg)
                    }
                }
                form method="post" action="/login" {
                    label for="password" class="field" {
                        "Password"
                        input type="password" id="password" name="password"
                            required autofocus autocomplete="current-password";
                    }
                    button type="submit" class="btn btn-primary" style="width:100%;" {
                        "Sign in"
                    }
                }
            }
        },
    )
}
