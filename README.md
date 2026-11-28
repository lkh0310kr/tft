# TFT

Rust/GPUI로 만든 네이티브 Markdown 지식 베이스입니다. 노트와 설정은 vault 폴더에서 직접 읽고 쓰며,
파일 사본을 따로 만들지 않습니다.

> TFT로 여는 vault는 백업해 두세요. 편집은 되돌릴 수 없는 파일을 다루는 비교적 새로운 코드입니다.

- 명세와 설계: [docs/Specification.md](docs/Specification.md)
- 웹과 Google Drive: [docs/Web.md](docs/Web.md), [docs/Web-Rebuild.md](docs/Web-Rebuild.md)
- Browser pane 계획: [docs/Browser.md](docs/Browser.md)

## 실행

vault 인자는 폴더, vault 안의 파일, 또는 `tft://` URI일 수 있습니다.

```sh
cargo run --release -p desktop -- /path/to/vault
cargo run --release -p desktop -- /path/to/vault/Note.md
cargo run --release -p desktop -- "/path/to/vault" --frame native
```

`tft --help`에 모든 flag가 있습니다. 한 vault 한 창, 두 번째 실행의 handover, `--control` socket 같은
동작은 [docs/Specification.md](docs/Specification.md) §8에 정리되어 있습니다.

## macOS 앱으로 설치

release 바이너리를 `TFT.app` 번들로 만들어 `/Applications`에 덮어쓰고 다시 실행합니다.

```sh
tools/macos-install.sh              # release build → /Applications/TFT.app → 앱 재실행
tools/macos-install.sh --no-launch  # 설치만
TFT_APP_DIR="$HOME/Applications" tools/macos-install.sh
TFT_KEEP_RUNNING=1 tools/macos-install.sh   # 다른 TFT 인스턴스를 닫지 않음
```

기존 앱과 다른 TFT 인스턴스(개발 빌드 등)를 먼저 종료하고, 번들을 ad-hoc 서명(hardened runtime)한
뒤 교체합니다. Apple Silicon에서 이모지 렌더링에 필요한 runtime flag가 여기 포함됩니다.

## 웹 (Google Drive)

현재 Web 대상은 `apps/web`의 GPUI WebAssembly 앱이고, `apps/web-leptos`는 보존된 Leptos 구현입니다.
Google Cloud/OAuth 설정과 Vercel 배포는 [docs/Web.md](docs/Web.md), 빌드와 브라우저 테스트는
[apps/web/README.md](apps/web/README.md)를 참고하세요.

## 빌드와 테스트

빌드·테스트·clippy·wasm-check 명령은 [AGENTS.md](AGENTS.md)의 Commands에 있습니다. CommonMark
corpus와 writer roundtrip 같은 호환성 기준은 [docs/Specification.md](docs/Specification.md) §9에
정리되어 있습니다.

## 라이선스

MIT. [LICENSE](LICENSE)를 참고하세요.
