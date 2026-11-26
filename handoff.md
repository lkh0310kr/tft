# TFT Web Implementation Handoff

## 제1목표

TFT의 Web 애플리케이션을 GPUI WebAssembly 기반으로 구현한다. Desktop과 Web에서 공통으로 쓸 수 있는 앱 동작과 편집 기능은 재사용하고, 브라우저 입력·클립보드·화면 등 플랫폼 차이는 GPUI Web에 연결한다. 목표는 별도의 축소판 웹 편집기가 아니라, Desktop과 같은 TFT 경험을 브라우저에서 제공하는 것이다.

iOS는 별도 네이티브 앱이 아닌 PWA로 지원하는 방향이지만, 이번 Web 구현의 우선 검증 대상은 데스크톱 브라우저다. iOS Safari/PWA의 입력 및 화면 동작은 이후 실기기 검증 단계에서 다룬다.

## 현재 위치

현재 목표인 **Web의 Desktop crate 의존 제거는 완료**했다. Web과 Desktop은 `crates/app`의 같은 production Editor/ReadingView를 사용하고, 플랫폼 차이는 Desktop의 `SuggestionSource`, clipboard file reader, ReadingView model builder adapter가 맡는다. 실제 Google 계정/Drive 왕복 검증은 사용자가 나중에 진행하기로 했다.

- 일반 WASM 빌드는 `apps/web::drive_app::DriveApp`을 띄워 OAuth/Picker 설정, Google 로그인, Drive 폴더 선택, Markdown 로드, `Save to Drive` 동작을 제공한다. `browser-test` feature는 회귀를 위해 fixture `WebWorkspace`를 사용한다.
- Drive Workspace에서 Cmd+S/macOS, Ctrl+S/Windows·Linux 단축키가 현재 Editor 원문을 저장하도록 연결했다. Drive mock 브라우저 흐름에서 단축키와 `Save to Drive` 버튼 모두 PATCH 원문을 확인했다.
- Desktop과 같이 마지막 입력 후 2초 debounce로 Drive 자동 저장을 연결했다. 변경 이벤트는 공유 Editor에서 VaultData/Search index로 반영되고, per-note 저장 세대와 ETag를 관리해 연속 편집을 직렬화한다. Mock Drive에서 debounce 저장 본문, Cmd+S, 수동 버튼과 무콘솔 오류를 검증했다.
- 변경된 문서가 저장 중이면 `Switch Vault`가 Workspace를 즉시 버리지 않고 해당 Drive 저장이 성공할 때까지 기다린다. 지연 저장 직전 Vault를 전환하는 브라우저 회귀에서 원문 PATCH 완료 후 Vault 화면이 바뀌는 것을 검증했다.
- `apps/web/src/web_workspace.rs`의 `WebWorkspace::from_vault`는 backend-neutral `VaultData`와 선택 노드에서 시작하고, Desktop과 같은 production `Editor`, `Theme`, `FileTree`, `WorkspaceModel`을 사용한다. 테스트용 문서 fixture 생성은 `browser-test` feature에만 있다.
- Files 헤더의 새 노트 버튼은 공통 Workspace 이벤트를 Drive host로 전달한다. root Vault에 Markdown 파일을 만들고 반환된 Drive ID로 같은 `VaultData` 그래프/검색 인덱스를 갱신한 뒤 새 노트를 공유 Editor/탭에서 연다. Drive mock 브라우저에서 생성 multipart 요청과 초기 원문을 확인했다. 현재는 root 폴더에 새 Markdown 노트 생성만 지원한다.
- 편집 중인 원문을 `VaultData`에 반영해 Desktop과 같은 `reading::build_with` 및 `ReadingView`로 Markdown 프리뷰를 전환할 수 있다. 브라우저 캡처에서 제목/문단/태스크/링크 스타일을 확인했고, Preview→Source 전환 후 편집 원문이 보존되는 회귀가 통과했다.
- Preview 태스크 토글은 production `Editor`의 task 명령으로 원문을 갱신한다. 일반 WikiLink는 공유 `WorkspaceModel`의 현재 탭 탐색 기록을 따라가고, Meta/Command+클릭은 대상이 이미 열려 있어도 별도 탭을 만든다. 링크 클릭, 뒤로/앞으로, task 원문 반영과 탭 전환 후 원문 보존을 Chrome에서 검증했다.
- Files pane 검색은 Desktop과 같은 `tft_search` matcher/snippet builder 및 공유 `tft_search::inputs` metadata 추출을 사용한다. Vault의 Markdown 본문/경로, tag, frontmatter property, section/task scope를 검색하고 결과를 공유 탭 모델로 연다. 검색 실행 전 현재 Editor 원문을 Vault와 metadata index에 갱신한다. ArrowUp/Down으로 결과를 선택하고 Enter로 여는 동작을 연결했다. Playwright에서 tag 검색, 다른 노트 검색, 편집 직후 검색, 키보드 결과 탐색을 검증했다.
- Explorer 도구막대의 정렬 메뉴(6개 `SortOrder`), 전체 접기/펼치기, 활성 문서의 상위 폴더 자동 펼치기를 공유 `FileTree`에 연결했다. fixture Vault 브라우저 회귀에서 정렬 상태와 표시 행을 검증했다.
- 파일 작업 경계를 정리했다. Desktop Explorer와 Web Workspace는 UI 이벤트가 달라도 공통 `tft_app::FileOperationRequest`를 통해 호스트에 작업 의도를 전달한다. 우선 `CreateNote { parent: Vault-relative path }`를 양쪽에 연결했다. 도메인의 `VaultData::validate_new_child_name`이 이름/형제 충돌 규칙을 소유하고, Desktop 로컬 파일시스템이나 Web Drive adapter는 경로를 저장소 객체 ID로 변환하고 I/O·충돌 재검사를 담당한다. 실제 Drive 검증은 보류한다.
- 공유 `WorkspaceModel` 탭 닫기 동작을 Web 탭 스트립에 연결했다. 활성 노트를 닫으면 남은 탭이 활성화되고 버퍼가 보존되는 Chrome 회귀가 통과했다.
- 탭 상태의 `Tab`/`HistoryEntry` 타입과 새 탭 삽입 규칙(`tft_app::place_new_tab`)은 Desktop과 Web이 공유한다. 탭 항목과 파일 행도 `ui::workspace_tab`/`ui::file_tree_row` 공통 GPUI 렌더러를 사용하며, Desktop의 실제 탭 스트립과 파일 탐색기도 이 렌더러를 쓴다. Web의 그룹·탐색 상태는 추가로 `WorkspaceModel`을 통해 관리한다.
- UI 포팅의 첫 조각으로 `crates/app::ui::workspace_view_header`를 추가해 문서 헤더의 높이·간격·폰트를 공유하고, Desktop Workspace와 WebWorkspace 둘 다 적용했다. Web에서 뒤로/앞으로 및 Source/Preview 동작을 탭 아래 문서 헤더로 옮겼고, 변경된 레이아웃에서 편집/탐색 브라우저 회귀가 통과했다. Desktop 및 Web fixture 캡처는 `artifacts/ui-screenshots/`에 있다.
- 검증 차이: Playwright headless Chrome에서 Web Workspace screenshot과 전체 브라우저 회귀가 통과했지만, 사용자 Chrome의 `127.0.0.1:8088` 페이지는 reload 후 검은 GPUI 캔버스로 보였다. 실 Chrome과 headless Playwright의 렌더 차이를 아직 설명하지 못했으므로 실 Chrome 표시를 완료로 간주하지 않는다.
- 파일 선택, 탭 열기/전환, 탭별 편집 내용 보존, 전환 시 편집 원문을 `VaultData`에 반영하는 동작이 Chrome에서 확인됐다.
- Google Drive 브라우저 API를 `crates/drive-web`으로 추출해 GPUI Web과 기존 브라우저 앱이 공유한다. 설정을 localStorage에 저장하고 reload 뒤 읽어 오는 동작을 Chrome에서 확인했다.
- 일반 DriveApp의 모의 브라우저 통합 테스트에서 GIS 토큰 콜백, Picker 폴더 응답, Drive 메타데이터/목록/미디어 GET, 원문 편집 후 업로드 PATCH 본문까지 확인했다. 실제 Google 계정 인증이나 Drive 파일 쓰기를 검증한 것은 아니다.
- Editor의 기존 브라우저 검증도 유지된다: 한글 글꼴, 더블클릭/드래그, Cmd/Option 이동, 복사·붙여넣기, Undo/Redo, ASCII 입력과 합성 한글 이벤트. 합성 이벤트는 실제 OS 한글 IME 검증이 아니다.
- `crates/app`은 Workspace 모델/명령, assets/theme/TextInput/UI helper와 production `Editor`, ReadingView, Markdown parse/render 및 spellcheck를 소유한다. Desktop은 이 공통 모듈을 re-export해 기존 호출부와 동일한 구현을 사용한다.
- `apps/web/Cargo.toml`과 Web 소스에서 `desktop` 의존을 제거했다. WASM `cargo tree`에도 Desktop package가 없다. DriveApp/WebWorkspace composition은 `apps/web`에 있고, `crates/drive-web`은 브라우저 Drive adapter다.
- Desktop `SuggestionSource`는 Vault 기반 suggestion과 block ID 쓰기를 담당한다. 공통 Editor에는 `VaultState` 직접 참조가 없다. Desktop만 clipboard file reader를 설정하고, ReadingView는 host가 제공하는 lazy model builder를 받는다.
- Desktop check, WebAssembly-only check, GPUI Web 편집/Drive mock 브라우저 회귀, `cargo test -p app --features gpui-ui`(230개), `app`/`domain` 테스트, Leptos WASM compile check가 통과한다. 실제 Google 계정/Drive 파일 왕복은 아직 검증하지 않았다.
- GPUI Web은 `apps/web`, 기존 Leptos 구현은 `apps/web-leptos`에 보존했다.

## 아직 구현되지 않은 범위

Google OAuth/Picker가 실제 계정으로 승인되는지, 실 Drive 파일을 읽고 저장한 뒤 재열 때 원문이 같은지는 아직 검증하지 않았다. 실제 Desktop Workspace의 도크 확장, 설정, 여러 그룹/분할, 파일 작업 메뉴, 그래프 등 다른 뷰는 GPUI Web에 아직 이식되지 않았다. 전체 Desktop과 동등한 TFT Web 앱이나 PWA 완성 상태는 아니다.
ReadingView의 Preview/Source 전환, Preview 태스크 토글, WikiLink 탐색이 WebWorkspace에 연결되어 있다. 표/속성 등 다른 Preview 편집 동작과 전체 Desktop Workspace 동등성은 남아 있다.

## Phase Plan

1. **기반 안정화 — 진행 중:** GPUI WASM 시작 수명 패치를 revision 검사 후 적용하는 `apps/web/build-web.sh`/`check-wasm.sh`/`serve-web.sh` 경로를 만들었다. 실제 upstream 반영 또는 유지 가능한 fork 전환은 남아 있다.
2. **앱/domain 분리 — 완료:** 공통 앱/Workspace 상태는 `crates/app`, Vault graph/identity는 `crates/domain`, GPUI shell은 각각 `apps/desktop`과 `apps/web`에 있다.
3. **공통 UI 경계와 Web Workspace — Editor 경계 완료:** Editor/ReadingView와 렌더러 의존 모듈을 `crates/app`으로 옮겼다. Web의 Desktop crate 의존은 제거됐다. Desktop 전체 Workspace orchestration을 Web과 통합하는 일은 별도 기능 확장 범위다.
4. **Web 저장소와 핵심 흐름 — 구현 완료, 실계정 검증 대기:** OAuth/Picker와 Drive 읽기·저장 흐름은 mock 브라우저 회귀를 통과했다. 실제 계정 왕복은 사용자가 별도로 집중 검증한다.
5. **기능 범위 확장 — 진행 중:** 기본 탐색기 도구 동작(정렬, 접기/펼치기, 현재 문서 reveal)을 fixture Vault로 검증했다. 다음은 파일 작업 메뉴와 Workspace 그룹/분할이다.
6. **PWA/iOS — 대기:** 데스크톱 브라우저 구현 이후 PWA 동작과 실기기 iOS Safari 입력/레이아웃을 검증한다.

입력 회귀 테스트는 공유 동작을 보장하는 수단이며 별도의 제품 목표로 확장하지 않는다.

## 검증된 기술 제약

- 고정된 GPUI revision의 Web 시작 경로는 비동기 실행 수명 문제로 RAF/DOM 콜백이 해제된 클로저를 호출한다. 현재 Web app은 이를 피하는 WASM 전용 패치를 Cargo 캐시에 적용한다. 제품 적용 전에는 수정된 upstream revision 또는 유지 가능한 패치 경로가 필요하다.
- 해당 의존성 구성의 WASM 빌드에는 `RUSTC_BOOTSTRAP=1`이 필요하다 (`wasm_thread`의 nightly feature 때문이다).
- GPUI Web의 기본 클립보드 경로는 PoC에서 브라우저 Clipboard API로 연결했다. 브라우저 키 별칭도 기존 Editor 명령에 매핑했다.
- 실제 OS IME, 키보드 가림, 터치 선택 핸들, 회전, safe area, PWA standalone 모드는 아직 검증하지 않았다.

## 빠른 확인

```sh
apps/web/check-wasm.sh
apps/web/build-web.sh
apps/web/serve-web.sh 8088
TRUNK_BIN=/path/to/trunk python3 apps/web/tests/test_browser.py
TRUNK_BIN=/path/to/trunk python3 apps/web/tests/test_drive_shell.py
TRUNK_BIN=/path/to/trunk python3 apps/web/tests/test_drive_flow.py
```

GPUI Web 앱의 구현 범위와 브라우저 테스트 설명은 `apps/web/README.md`를 참고한다.
