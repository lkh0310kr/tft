---
aliases:
  - Spec
tags:
  - tft/meta
---
# TFT 프로젝트 명세

TFT는 Markdown 노트를 Vault에서 직접 다루는 Rust/GPUI 기반 지식 관리 앱입니다. 노트 파일은 원래 위치에서 열고 저장하며 별도 사본을 만들지 않습니다. 앱 설정은 Vault의 `.tft/`에 저장하고, 다른 앱의 설정을 가져오거나 마이그레이션하지 않습니다. 이 문서는 프로젝트가 구현하는 동작의 기준을 정의합니다. 측정 기록은 `tools/reference-probe/reference/observed-behavior.md`에 있습니다. 이 명세와 프로브 결과가 다르면 프로브를 우선하고 명세를 수정합니다.

## 1. 불변 조건

1. **Vault 폴더가 원본입니다.** TFT는 변경하지 않은 파일을 쓰지 않습니다. 줄바꿈, BOM, JSON key 순서, 알 수 없는 YAML/JSON key, 마지막 줄바꿈 여부는 읽고 쓴 뒤에도 보존됩니다.
2. **Markdown parser는 하나만 사용합니다.** `markdown` (`tft_markdown`)이 Editor, reading view, metadata cache, link rewriting에 데이터를 공급합니다. 이 용도에 별도 tokenizer를 추가하지 않습니다.
3. **파일 형식 식별자는 정확히 일치해야 합니다.** Command ID, `.tft` 파일명과 config key, `workspace.json` 노드 구조, Canvas JSON, hotkey 표기는 측정된 형식에 맞춥니다. 전체 목록은 `crates/commands/data/commands.txt`에 있습니다.
4. **Clean room 구현입니다.** 동작은 프로브 기록과 공개 문서를 바탕으로 구현합니다. 외부 앱의 코드, CSS, 아이콘을 복사하지 않습니다.
5. **공유 crate에서 네이티브 플랫폼 API를 사용하지 않습니다.** GPUI 앱 진입점은 `apps/desktop`과 `apps/web`에 둡니다. `crates/app`과 `crates/domain`은 window에 종속되지 않으며 플랫폼 runner 없이 테스트할 수 있어야 합니다.

## 2. 저장소 구조

```
apps/desktop        GPUI Desktop 앱 (package desktop; binary tft)
apps/web            GPUI WebAssembly 앱 (Google Drive host)
apps/web-leptos     보존된 Leptos 구현
assets              Desktop과 향후 Web 앱에서 공유하는 아이콘과 폰트
crates/app          공유 application 상태/use case 및 WASM 호환 서비스
crates/domain       정본 VaultData 그래프, domain 식별자와 불변 조건
crates/vault        Vault 스캔, 파일 종류, .tft 읽기/쓰기
crates/markdown     CommonMark + GFM + TFT Markdown 확장, byte span을 포함한 단일 tree
crates/cache        CachedMetadata와 link index/resolver
crates/formats      디스크 형식: config, Canvas, property, migration
crates/commands     command ID, registry, 기본 hotkey
crates/text         Editor text model: buffer, selection, undo, edit plan
crates/editor…      (desktop 내부) Editor view, Live Preview, table
crates/suggest      completion trigger, 후보, 확정된 link 문자열
crates/search       query 문법, matcher, fuzzy search, 제외 규칙
crates/canvas       Canvas viewport/geometry/hit-test/edit model
crates/pdf          PDF 페이지, outline, text, #page= 하위 경로 (hayro)
crates/math         RaTeX 기반 TeX 조판 (KaTeX 호환)
crates/mermaid      limu를 통한 Mermaid 레이아웃
crates/highlight    Prism tokenizer와 85개 언어 grammar
crates/export       PDF 내보내기 (krilla), 이미지 내보내기 (tiny-skia)
crates/audio        오디오 재생과 비디오 헤더
crates/paste        첨부 파일명, text/uri-list 디코딩
crates/spell        Hunspell 맞춤법 검사, 사전, 사용자 단어
crates/control      control socket wire protocol 및 blocking client
crates/cli          인자/URI 처리, 단일 창 전달
crates/bench        합성 Vault 생성기, 색인 시간 측정
```

package 및 디렉터리명에 `tft_` 접두어가 없더라도 Rust library 이름은 `tft_*`를 유지합니다. `use tft_text::…` import를 안정적으로 유지하고 앱 자체 module(`control`, `search`, `canvas`, `pdf`, `math`, `mermaid`, `suggest`)과 이름이 충돌하지 않게 하기 위해서입니다.

`core`, `markdown`, `formats`, `cache`, `text`, `search`, `suggest`, `canvas`, `commands`, `highlight`, `paste`, `pdf`, `math`, `mermaid`, `export`는 `wasm32-unknown-unknown`으로 빌드됩니다. 해당 경로는 `cargo wasm-check`로 검사합니다. `vault`, `audio`, `spell`, `control`, `cli`, `apps/desktop`은 native 환경을 대상으로 합니다. Web 앱은 동일한 순수 crate를 사용하며, Google Drive는 Desktop 저장소가 아니라 Web 앱의 저장소입니다.

웹 셸은 기능을 축소해 별도로 재구현하지 않습니다. 브라우저 뷰포트 안에서 동일한 `core::design` token과 치수, file tree, tab group, reading/source view, Editor 동작을 사용합니다. 브라우저 UI만 피할 수 없는 차이입니다. 공유 UI 데이터는 `core`에 두고, DOM widget은 같은 명세를 따르는 두 번째 구현체이지 별개의 제품이 아닙니다.

## 3. Vault와 설정

- Vault는 폴더입니다. `.tft/`이 있으면 읽고, 없으면 기본값을 사용합니다.
- 모델링한 config 파일의 알 수 없는 key를 모두 보존합니다. 설정을 쓸 때는 해당 동작으로 바뀐 key만 수정합니다.
- 모델링 대상 파일: `app.json`, `appearance.json`, `workspace.json`, `hotkeys.json`, `graph.json`, `types.json`, `switcher.json`, `page-preview.json`, `canvas.json`, `core-plugins.json`.
- **Core plugin은 항상 활성화합니다.** 앱은 더 이상 `core_plugins.json`을 읽지 않으며 Core plugins 설정 페이지도 없습니다. plugin 동작 코드는 계속 모듈화합니다.
- **설정은 다음 TFT 기본값으로 고정합니다.**
  - 새 노트: Vault root
  - 첨부 파일: Vault root
  - link 형식: shortest path
  - 이름 변경/이동 시 내부 link를 업데이트하기 전에 사용자에게 확인
  - `[[Wikilinks]]` 활성화
  - 제외 파일 규칙은 비어 있으며 적용하지 않음
  - 삭제한 파일은 시스템 휴지통으로 이동
  - 삭제 전 확인 활성화
- `workspace.json`은 탭/분할 트리를 기록합니다. `"stacked": true`인 group은 가로 탭으로 읽고, 다음 저장 때 해당 key를 제거합니다.
- Theme 출력은 dark/light 두 팔레트(131 token)로 고정합니다. `theme.css`, community theme, snippet, accent, font, app icon 사용자 지정은 지원하지 않습니다. Appearance 페이지는 Light/Dark/System만 제공합니다.

## 4. Markdown과 읽기

`tft-markdown`은 CommonMark(표준 예제 652개 전체), GitHub 확장, TFT Markdown 문법을 byte span이 포함된 단일 tree로 파싱합니다. 같은 tree를 reading view, cache, Editor decoration, link rewriting에 사용합니다.

- Callout, wikilink (`[[Note]]`, `[[Note#Heading]]`, `[[Note^block]]`, alias와 하위 경로), embed (`![[…]]`), tag, math (`$x$`, `$$x$$`), fenced code, task list, table, frontmatter, footnote/definition, comment, raw HTML, Markdown 확장을 지원합니다.
- Reading view는 tree를 HTML로 렌더링합니다. `crates/markdown/tests/corpus`의 측정 결과를 회귀 기준으로 사용합니다.
- 이미지 embed의 width/height, 중첩 목록/인용문, 원격 `http(s)` 이미지, PDF 하위 경로를 처리합니다.
- Code fence는 fence에 지정된 언어의 Prism grammar로 강조합니다. 알 수 없는 언어는 일반 코드로 표시합니다.

## 5. 편집

Editor는 `tft-text`를 사용하는 단일 view입니다. buffer, selection, undo, edit plan은 해당 crate에서 관리합니다.

- **Reading / Live Preview / Source** 모드를 제공합니다. 새 탭은 Editing view와 Live Preview로 시작합니다. Source 모드는 문법 강조가 적용된 Markdown 원문을 표시합니다.
- Find/replace, multiple cursor, 괄호와 Markdown 문법 자동 완성, 스마트 목록 이어쓰기, 폭 4의 tab 들여쓰기, 읽기 편한 줄 길이, heading/list 접기 기능을 제공합니다. 줄 번호는 끄고 들여쓰기 가이드는 켭니다. 오른쪽에서 왼쪽으로 쓰는 텍스트는 기본 비활성화합니다.
- Table: 셀 편집, 행/열 명령, 정렬, 드래그 handle을 제공합니다.
- Property는 노트 상단에 표시되는 편집 가능한 typed table입니다. 편집할 때 변경된 줄만 다시 씁니다. YAML type은 `types.json`에 저장합니다.
- 이름 변경/이동/복사 시 위에 정의한 shortest-path, wikilink, 업데이트 전 확인 규칙에 따라 link를 수정합니다.
- 자동 완성: `[[`, `![[`, `#`, slash command를 입력하면 파일, alias, heading, tag, block reference를 제안합니다.
- Paste/drop: 파일과 clipboard 이미지는 Vault root에 첨부 파일로 저장합니다. 붙여 넣은 이미지는 `Pasted image <local yyyyMMddHHmmss>.png`로 이름을 지정하며, 여러 파일을 놓으면 입력 순서대로 embed합니다. **복사한 HTML은 변환하지 않습니다.** 웹에서 붙여넣으면 clipboard의 plain text를 씁니다.
- **Vim key binding은 지원하지 않습니다.**

## 6. 앱 셸

- 기본은 frameless window이며 TFT 자체 title row, window button, 창 이동/확대, window menu를 사용합니다. `--frame native`를 지정하면 Desktop의 native frame을 사용합니다.
- 왼쪽 sidebar에는 **Files**가 있습니다. 전체 검색은 `Mod+Shift+F`로 중앙 **Search** 탭을 엽니다. 왼쪽 ribbon은 제거했습니다.
- 메인 영역은 가로 스크롤이 가능한 tab strip과 새 탭 버튼, tab-list 메뉴로 구성합니다. 탭을 오른쪽/아래로 분할하고, 드래그하고, 고정하고, 별도 창으로 분리하고, `workspace.json`에서 복원할 수 있습니다. stacked tab은 지원하지 않으며 모든 group은 가로 탭입니다.
- 오른쪽 dock(Outgoing links, Tags, All properties, Outline, File properties)을 제거했습니다.
- 오른쪽 아래 status bar는 없습니다(파일명, Ln/Col, property 수를 표시하지 않음).
- Files는 폴더별 펼치기/접기와 파일 탐색을 제공합니다. New note/New folder header 버튼은 제거했습니다.
- Settings는 하나의 스크롤 페이지를 가진 단일 창입니다. **Appearance**(기본 색상 scheme)와 **Editor**를 제공합니다. General/Hotkeys/Core plugins/Files and links 탭과 왼쪽 Settings sidebar는 제거했습니다.
- Notice는 잠시 표시됩니다. 상태 텍스트는 control socket에서 계속 확인할 수 있습니다.

## 7. 뷰

- **Files** — 폴더 트리, 키보드 탐색, context menu, 행 내 이름 변경, drag/drop, 파일 작업. 파일·폴더·빈 영역의 context menu는 **같은 항목 목록**을 아이콘 없이 보여 주고, 그 대상에 쓸 수 없는 항목은 disabled(흐리게)로 둡니다(Zed 방식). 파일 행의 New note/New folder/New canvas와 Search in folder는 그 파일이 있는 **부모 폴더**를 대상으로 합니다.
- **Search** — `Mod+Shift+F`로 중앙 Search 탭을 엽니다. query 문법(`path:`, `file:`, `tag:`, `line:`, `section:`, `block:`, `task:`, regex, 대소문자/설명 옵션), 강조 표시가 있는 그룹 결과, Copy search results를 지원합니다. 결과는 일반 문서 탭에서 열리고 Search 탭은 유지됩니다.
- **Quick switcher** — `Mod+P`(macOS `Cmd+P`)로 열고, 파일/alias fuzzy 탐색. 존재하는 파일만 열며, 일치하는 파일이 없거나 "Not created yet" 대상이면 Enter를 눌러도 새 파일을 만들지 않습니다.
- **Command palette** — `Mod+Shift+P`(macOS `Cmd+Shift+P`)로 열고, 지금 실행할 수 있는 명령을 fuzzy 탐색합니다. 빈 query는 이 세션에서 최근 실행한 명령부터 나열하며, 각 행에 hotkey를 표시합니다. Slash commands popup과 같은 command list를 사용합니다.
- **Graph** — 전체/로컬 graph, filter, group, force, timelapse; 첨부 파일은 선택 사항.
- **Canvas** — 이동/확대/선택/이동/크기 조절, card, note/image/PDF card, label이 있는 edge, group, 정렬, undo. `.canvas` JSON은 정해진 byte 형식으로 저장합니다.
- **PDF** — hayro 렌더링, toolbar, sidebar thumbnail, 검색 bar, text 선택, spread, `![[x.pdf]]` embed와 `#page=` 하위 경로.
- **Media** — 오디오 재생과 비디오 header.
- **Math** — RaTeX/KaTeX 호환 display 및 inline 수식.
- **Mermaid** — limu로 diagram을 배치하고 drawing list로 그립니다.
- **Export** — PDF 내보내기와 Canvas 이미지 내보내기. GPUI 비종속 layout engine, krilla, tiny-skia를 사용하며 핵심 앱의 필수 기능은 아닙니다.

## 8. 명령, 단축키, 자동화

- TFT command ID가 API입니다. `hotkeys.json`의 사용자 지정 binding은 기본 binding을 덮어쓰되 원문 그대로 보존합니다. 제거된 기능의 command ID는 목록에서 제외합니다.
- **Modifier 정책**: `hotkeys.json`과 커맨드 데이터는 Obsidian 표기(`Mod`, `Ctrl`, `Alt`, `Shift`, `Meta`)를 씁니다. `Mod`는 macOS에서 `Cmd`, Windows/Linux에서 `Ctrl`이며, 런타임 바인딩은 `crates/app/src/keymap.rs`를 통해 GPUI `secondary`로만 겁니다. 리터럴 `Ctrl`은 Obsidian이 모든 플랫폼에서 Ctrl로 쓰는 키(탭 전환 `Ctrl+Tab`/`Ctrl+PageUp`/`Ctrl+PageDown`, 제안 팝업 `Ctrl+N`/`Ctrl+P`, 입력 필드의 `Ctrl+E`/`Ctrl+U`)에만 남깁니다. 단어 이동/삭제는 macOS `Option`, 그 외 `Ctrl`; 줄 끝은 `Home`/`End`와 macOS `Cmd+←/→`; 문서 끝은 macOS `Cmd+↑/↓`, 그 외 `Ctrl+Home`/`Ctrl+End`입니다. wasm 빌드는 브라우저의 OS를 알 수 없으므로 `Mod` 키를 `Cmd` 표기로도 함께 바인딩합니다.
- `app:open-settings`, `workspace:new-tab`, 분할/포커스/탭 명령, 파일 작업, Editor 서식 및 표 명령, 검색/빠른 전환/Graph/Canvas/PDF 명령을 유지합니다.
- 탭 번호 이동(`workspace:goto-tab-*`), 표 편집(`editor:table-*`), 분할 포커스(`editor:focus-*`)는 기능과 단축키는 유지하되 커맨드 팔레트와 slash popup 목록에서는 숨깁니다(`PALETTE_HIDDEN`).
- CLI는 Vault, Vault 내 파일, `tft://` URI를 받을 수 있습니다. Vault 하나당 창 하나를 사용하며, 두 번째 실행은 기존 프로세스에 요청을 전달합니다. `--control <socket>`은 상태, 열기, mode, key, mouse, 파일, command, 종료를 다루는 newline-delimited JSON control socket을 노출합니다. 모든 플랫폼에서 Unix domain socket을 사용합니다. 녹화도 이 인터페이스로 수행합니다.
- `crates/control`은 wire client이며 `apps/desktop/src/control.rs`는 server입니다.

## 9. 테스트와 호환성

- `cargo test --workspace`는 unit, corpus, integration test를 실행합니다.
- Parser 회귀 기준: CommonMark 예제 652개와 1000개 노트 rendering corpus.
- Writer 회귀 기준: `TFT_ROUNDTRIP_VAULT=<vault> cargo test -p text --test writer_roundtrip`은 모든 노트를 열고 byte 단위로 동일하게 다시 저장하며 CRLF/BOM/frontmatter 사례도 검사합니다.
- `crates/*/tests/data`의 format fixture는 측정한 byte를 보관합니다. 정규화하지 마세요.
- 앱과 library는 Linux, Windows, macOS에서 빌드합니다. gpui는 Zed release commit으로 고정되어 있으며 버전을 올리기 전에 실제 GPU에서 검증해야 합니다.

## 10. 의도적으로  제외한 기능

Bases, Daily notes, Sync, Note composer, File recovery, Show debug info, Reload app, Sandbox vault, Frontmatter migration, Show release notes, Graph timelapse, Canvas jump-to-group/convert-to-file, Window always-on-top, 색상 하이라이트 6종, Cycle bullet/checkbox, Copy TFT URL, Create note in current tab/to the right, Open another vault, Templates/Unique note creator, Publish, Web viewer, Audio recorder, Footnotes pane, Slides, Word count, Workspaces, Bookmarks, Backlinks UI, Random note, stacked tab, tab-list chevron menu, ribbon, 오른쪽 dock, 왼쪽 Settings sidebar, Hotkeys 탭, plugin별 옵션 탭, Vim key binding, HTML→Markdown paste, community theme/snippet/font, 사용자 지정 app icon은 제외합니다. git history에서 이 기능을 복원하지 않습니다.
