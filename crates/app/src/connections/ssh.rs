//! The dialog's SSH section: the tunnel on or off, the server, its user and
//! how it signs in. Every field stays in place whatever the method, so
//! switching never moves the form; while the tunnel is off they are disabled.

use dbforge_core::connections::transport::SshAuth;
use gpui::{Context, Div, ParentElement, Styled, Window, div, px};

use super::dialog::ConnectionDialog;
use super::form::{TextField, is_valid_port, tunnel_unreachable};
use super::general::{host_row, pair};
use crate::theme::ActiveTheme;
use crate::ui::Checkbox;
use crate::ui::field::{field, help, label};

impl ConnectionDialog {
    pub(super) fn render_ssh(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let m = Self::messages(cx);
        let ssh = &self.form.ssh;
        let unreachable = tunnel_unreachable(&self.form);
        let toggle = Checkbox::new("use-ssh", m.use_ssh, ssh.enabled)
            .help(if unreachable { m.ssh_needs_host } else { m.use_ssh_help }, unreachable.then_some(theme.warn))
            .on_change(cx.listener(|this, enabled: &bool, window, cx| {
                let enabled = *enabled;
                this.change(|form| form.ssh.enabled = enabled, window, cx);
            }));
        let secret_label = if ssh.auth == SshAuth::Key { m.ssh_passphrase } else { m.ssh_password };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(toggle)
            .child(host_row(
                self.text_field(TextField::SshHost, m.ssh_host, Some(m.ssh_host_info), false, window, cx),
                self.text_field(
                    TextField::SshPort,
                    m.port,
                    Some(m.ssh_port_info),
                    ssh.enabled && !is_valid_port(&ssh.port),
                    window,
                    cx,
                ),
            ))
            .child(pair(
                self.text_field(TextField::SshUser, m.ssh_user, None, false, window, cx),
                field(label(&theme, "info-ssh-auth", m.ssh_auth, Some(m.ssh_auth_info)), self.ssh_auth.clone(), None),
            ))
            .child(self.file_field(TextField::SshKeyFile, m.ssh_key_file, m.ssh_key_file_info, None, cx))
            .child(self.text_field(TextField::SshSecret, secret_label, Some(m.ssh_secret_info), false, window, cx))
            .child(help(&theme, m.host_keys_help, None).mt(px(0.)))
    }
}
