# TFT

Rust/GPUI 네이티브 Markdown 지식 베이스입니다. Vault 폴더를 유일한 원본으로 유지합니다.

- 불변 조건(원본 보존, 단일 Markdown parser, 측정된 식별자, clean room): `docs/Specification.md` §1
- 전체 앱/crate 구조: `docs/Specification.md` §2

## 구조 보충

Specification §2에 없는 항목:

- `docs/` — TFT로 직접 여는 문서 vault.
- `tools/prism-port` — Prism 1.29에서 `highlight`의 grammar와 corpus를 생성합니다(node). 손으로 고치지 말고
  스크립트를 고친 뒤 다시 실행합니다.
- `tools/plugin-spike` — wasmtime vs extism을 측정한 phase 5 runtime spike. Cargo workspace 밖에 있습니다.
- `tools/writer-gate` — Writer gate baseline/diff. `gate.sh check`, `gate.sh baseline`, `gate.sh status`.
- `tools/reference-probe` — probe 스크립트. `reference/observed-behavior.md`가 명세의 근거인 원시 관측
  기록입니다.

## Commands

Linux/NixOS에서는 `nix-shell` 안에서 실행합니다. `shell.nix`가 gpui의 X11, Wayland, Vulkan 라이브러리를
제공합니다. `CARGO_TARGET_DIR`은 기본으로 저장소의 `target/`을 쓰며, 저장소가 SMB 공유에 있어 Linux
머신도 같은 곳에서 빌드합니다.

Mac에서는 `~/.cargo/config.toml`의 `build.build-dir`로 중간 산출물을 로컬 디스크에 둡니다. smbfs에는
fcntl lock이 없어 증분 컴파일이 되지 않기 때문입니다. 공유가 다시 quarantine 플래그로 마운트되면
공유에 쓰인 바이너리가 Gatekeeper 프롬프트를 띄우므로, 그때는 `CARGO_TARGET_DIR`을 로컬 디렉터리로
지정합니다.

Apple Silicon에서 `cargo`가 링커로 서명한 바이너리에는 hardened runtime flag가 없어, Apple Color Emoji를
그리는 순간 CoreText가 `CopyEmojiImage`에서 SIGBUS로 죽습니다. `.cargo/config.toml`의 runner가
`cargo run`/`cargo test`가 시작하는 바이너리를 `--options runtime`으로 다시 서명합니다. `cargo build`
결과를 직접 실행할 때는 `tools/macos-run.sh target/debug/tft …`처럼 실행합니다.

```
cargo test --workspace            # 라이브러리 전체 테스트 (빠름)
cargo wasm-check                  # 순수 crate를 wasm32-unknown-unknown으로 검사
TFT_ROUNDTRIP_VAULT=<vault> cargo test -p text --test writer_roundtrip   # 실제 vault로 Writer gate roundtrip
cargo check -p desktop             # 앱 확인 (gpui 첫 빌드는 느림)
cargo run --release -p desktop -- <vault> [--note "Name#Heading"]   # vault 열기 (release: 편집마다 재파싱)
cargo run --release -p desktop -- --help                            # cli 표에서 생성한 모든 flag
cargo run --release -p desktop -- <vault>/Note.md --new-window      # 파일 인자; --new-window는 두 번째 실행의 handover를 건너뜀
cargo run --release -p desktop -- <vault> --frame native   # hidden frame 대신 데스크톱 title bar (TFT_FRAME=native와 동일)
cargo run --release -p desktop -- <vault> --synthetic 50000 --stress   # 에디터 spike: 자체 편집을 돌리고 시간 출력
cargo run --release -p desktop -- <vault> --note X --source --type '[[al' [--accept enter]  # 자동완성 popup 열기(및 키 입력)로 스크린샷
cargo run --release -p bench -- generate <dir>   # 합성 20k-note vault
cargo run --release -p bench -- index <vault>    # scan/read/index/resolve 시간 측정, 2초 gate 확인
cargo fmt --all && cargo clippy --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings   # CI와 동일: 현재 stable, warning 거부
```

## Conventions

- Vault 기준 경로는 `/`를 쓰고 원래 대소문자를 유지합니다. 비교는 대소문자를 무시합니다.
- 위치는 0부터 시작하는 line/column과 byte offset을 함께 사용합니다.
- Vault가 필요한 테스트는 `tempfile::tempdir()`에 직접 만듭니다. 개발자의 vault에 의존하지 않습니다.
- 모델링하지 않은 JSON/YAML key 보존은 `#[serde(flatten)] extra`로 구현합니다.
- `crates/markdown/tests/corpus`의 렌더링이 달라지면 `tests/corpus/vault/edge`에 probe note를 추가하고,
  그 vault의 plugin으로 다시 렌더링해 관측된 동작에 맞춥니다. 결과는 `docs/Specification.md`에
  기록합니다. CommonMark suite와 corpus는 ratchet입니다.
- `docs/`는 Markdown vault입니다. wikilink와 frontmatter로 노트를 쓰고, `.tft/`는 `workspace.json`만
  빼고 commit합니다.
