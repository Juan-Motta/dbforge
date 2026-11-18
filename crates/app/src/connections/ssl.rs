//! The dialog's SSL section: the mode, with what it checks (or why it can't
//! work through the tunnel), and the certificate files the engine can use.

use dbforge_core::connections::transport::SslMode;
use gpui::{Context, Div, ParentElement, Styled, Window, div, px};

use super::dialog::ConnectionDialog;
use super::form::{TextField, ssl_blocked_by_tunnel};
use crate::i18n::ConnectionMessages;
use crate::theme::ActiveTheme;
use crate::ui::field::{field, help, label};

pub(super) fn mode_label(mode: SslMode, m: &ConnectionMessages) -> &'static str {
    match mode {
        SslMode::Disable => m.ssl_disable,
        SslMode::Prefer => m.ssl_prefer,
        SslMode::Require => m.ssl_require,
        SslMode::VerifyCa => m.ssl_verify_ca,
        SslMode::VerifyFull => m.ssl_verify_full,
    }
}

fn mode_help(mode: SslMode, m: &ConnectionMessages) -> &'static str {
    match mode {
        SslMode::Disable => m.ssl_disable_help,
        SslMode::Prefer => m.ssl_prefer_help,
        SslMode::Require => m.ssl_require_help,
        SslMode::VerifyCa => m.ssl_verify_ca_help,
        SslMode::VerifyFull => m.ssl_verify_full_help,
    }
}

impl ConnectionDialog {
    pub(super) fn render_ssl(&self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let m = Self::messages(cx);
        let form = &self.form;
        let support = form.engine.ssl_support();
        let blocked = ssl_blocked_by_tunnel(form);
        let note = if blocked {
            help(&theme, (m.verify_full_through_ssh)(form.engine.label()), Some(theme.warn))
        } else {
            help(&theme, mode_help(form.ssl.mode, m), None)
        };
        // Two lines are kept for whichever text shows, so changing the mode never moves the fields.
        let mode = field(
            label(&theme, "info-ssl-mode", m.ssl_mode, Some(m.ssl_mode_info)),
            self.ssl_mode.clone(),
            Some(note.min_h(px(32.))),
        );
        let mut panel = div().flex().flex_col().gap(px(16.)).child(mode);
        if support.ca_file {
            panel = panel.child(self.file_field(TextField::CaFile, m.ca_file, m.ca_file_info, None, cx));
        }
        if support.client_cert {
            panel = if support.separate_key {
                panel.child(self.file_field(TextField::CertFile, m.client_cert, m.client_cert_info, None, cx)).child(
                    self.file_field(
                        TextField::KeyFile,
                        m.client_key,
                        m.client_key_info,
                        Some(help(&theme, m.client_key_help, None)),
                        cx,
                    ),
                )
            } else {
                panel.child(self.file_field(
                    TextField::CertFile,
                    m.mongo_client_cert,
                    m.mongo_client_cert_info,
                    None,
                    cx,
                ))
            };
        }
        panel
    }
}
