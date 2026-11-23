# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub's vulnerability reporting:
[github.com/lkh0310kr/tft/security/advisories/new](https://github.com/lkh0310kr/tft/security/advisories/new).
Do not open a public issue, pull request or discussion for one.

Please include the TFT version (or commit), your platform, how to reproduce it — a vault, note,
config file, canvas, PDF or socket request that triggers it — and what an attacker gains.

What to expect: an acknowledgement within 7 days, a fix in a tagged release, and credit in its
notes unless you would rather not be named. Please allow 90 days, or until the fix ships, before
disclosing publicly; if a fix needs longer you will hear why and we agree a date.

## What TFT promises

TFT opens a folder and treats it as the only thing it may change. A report is measured against
these properties:

- **Vault writes stay inside the vault.** Writes go through the vault's checked path: `..` and
  absolute paths are refused, as is a symbolic link anywhere along the way, and files are opened
  without following a link in their place (`O_NOFOLLOW` on Unix). The only write outside the
  folder is moving a deleted file to the system trash when that is the selected trash option.
- **No code from the vault runs.** There is no community-plugin runtime, no theme CSS and no
  snippet execution. Markdown, configuration and `.canvas` files are data.
- **The control socket is local and private.** `--control <path>` binds a Unix-domain socket with
  owner-only permissions and accepts no path that leaves the vault. It is off unless asked for,
  and it is as powerful as the keyboard: anyone who can connect to it can edit the vault.
- **Bytes TFT did not change are left alone.** Config files keep unknown keys, byte-order marks
  and line endings; a note that changed on disk underneath an open buffer is not overwritten.
- **Remote images are bounded.** `http:`/`https:` images and explicit attachment downloads are
  fetched without cookies, credentials or a referrer, with a 64 MB body limit and a timeout.
  A malicious vault can make TFT issue GET requests to hosts named in its notes, including hosts
  on the local network; that is the same behavior Obsidian has.

## Scope

In scope: the `tft` binary and release archives, the control socket, and anything a malicious
vault, note, config file, canvas, PDF or link can make the app do — write outside the folder,
execute code, expose local data, or lose unsaved edits.

Out of scope: problems that require an attacker who already controls the user account or the
vault folder's permissions, and vulnerabilities in Obsidian itself.

## Supported versions

Only the latest tagged release receives fixes. Binaries are built by the release workflows in this
repository; see [README](README.md#releases) for how to verify a download.
