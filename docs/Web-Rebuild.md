---
tags:
  - tft/meta
  - tft/plan
---
# GPUI Web implementation

## 제1목표

TFT의 Web 앱을 GPUI WebAssembly로 구현한다. Desktop에서 Web과 공통으로 쓸 수 있는 UI와 앱 동작을 재사용하고, 브라우저 플랫폼 차이는 GPUI Web 경계에 연결한다. Leptos로 별도 렌더러를 확장하는 것이 이번 목표는 아니다. iOS는 PWA로 지원하되 데스크톱 브라우저의 Web 구현 이후에 검증한다.

## Phase Plan

### Phase 0 — GPUI Web 실행 기반 (진행 중)

- GPUI WASM 시작 수명 문제의 revision 검사 패치와 공통 `build-web.sh`, `check-wasm.sh`, `serve-web.sh` 진입점을 유지한다.
- 현재 patch runner는 Cargo 캐시의 pinned GPUI 소스를 수정한다. 안정적인 제품 적용 전에 유지 가능한 pinned revision/fork 경로를 정한다.
- Gate: clean checkout에서 패치 적용, WASM 빌드, 브라우저 실행을 반복할 수 있어야 한다.

### Phase 1 — 공통 UI 경계와 Workspace 셸 (진행 중)

- GPUI 공통 Editor/ReadingView/테마/입력/렌더링을 `crates/app`에 둔다. OS 기능은 앱 shell host adapter로 연결한다.
- Web에서 같은 `Editor`, `FileTree` 모델, 탭 모델, 탭/행 GPUI 렌더러와 테마를 조합하고 Workspace 동작을 확장한다.
- Gate: 브라우저에서 파일 선택, 탭 열기/전환, 편집 버퍼 보존이 가능하고 Desktop 빌드/동작이 유지되며 Web dependency tree에 Desktop crate가 없어야 한다. **통과:** `apps/web/Cargo.toml`은 `app/domain/drive-web`을 사용하고 `cargo tree`에 Desktop이 없다.

### Phase 2 — 실제 Drive 검증 (대기)

- 공통 코드 구조 정리 뒤 실제 Google 계정으로 OAuth/Picker 및 Vault 선택을 검증한다.
- 실제 Drive 파일을 열고 수정·저장·재열기해 원문 일치와 충돌 처리를 확인한다.
- Gate: 브라우저에서 Drive Markdown을 열고 수정·저장 후 다시 읽었을 때 원문이 일치한다.

### Phase 3 — TFT Web 기능 확장 (대기)

- 파일 탐색, 읽기, 검색, 여러 탭과 기타 합의된 Desktop 기능을 공유 UI 경계에 연결한다.
- Gate: 실제 브라우저에서 승인된 핵심 TFT 작업 흐름을 수행하고 Desktop과 문서 결과 및 앱 상태가 일치한다.

### Phase 4 — PWA/iOS (대기)

- 데스크톱 브라우저 구현 뒤 PWA standalone 및 실기기 iOS Safari의 IME, 터치 선택, 키보드 가림과 safe area를 검증한다.

## 현재 상태

GPUI Web은 별도 `apps/web` 진입점에서 동작한다. Desktop crate dependency를 제거했고, production Editor/ReadingView/Workspace 모델은 `crates/app`에서 공유한다. Desktop 전체 Workspace orchestration만 Web과 다르다.

- `crates/app`의 production `Editor`/ReadingView가 Desktop과 Web 양쪽에서 빌드되고 GPUI Web 브라우저에서 실행된다.
- `apps/web/src/web_workspace.rs`의 `WebWorkspace`가 production `Editor`, ReadingView, 공유 Theme, `tft_app::explorer::FileTree`, `tft_app::WorkspaceModel`, `VaultData`를 조합한다. Editor와 ReadingView는 `tft_app`에서 직접 가져오며 Desktop crate import는 없다. 제품 초기화 경계는 backend-neutral `from_vault(VaultData, initial_node)`이며, 문서 fixture 생성은 browser-test 전용이다. Drive host와 Workspace composition은 Web 앱에 있고 storage는 `crates/drive-web`을 직접 사용한다. 탭 셸 `ui::workspace_tab`과 파일 행 `ui::file_tree_row`는 Desktop의 실제 Workspace/Explorer에서도 사용한다.
- Desktop과 Web은 공유 `Tab`/`HistoryEntry` 상태와 `tft_app::place_new_tab` 삽입 규칙을 사용한다. Web은 추가로 `WorkspaceModel`이 그룹·탐색 상태를 소유한다. Desktop의 더 큰 Workspace 명령·도크·그룹 orchestration은 아직 자체 상태를 유지한다.
- WebWorkspace는 편집 중인 원문으로 Desktop과 같은 `reading::build_with` 및 `ReadingView`를 사용해 Preview/Source를 전환한다. 링크의 빠른 down/up 입력은 공유 `LinkMenu`에서 프레임 간 상태를 보존한다. 일반 WikiLink는 현재 탭 기록에 추가하고, Meta/Command+클릭은 대상이 이미 열려 있어도 새 탭을 만든다. Preview 태스크 토글과 Back/Forward도 공통 `Editor`/`WorkspaceModel`에 연결했다. Playwright에서 링크 탐색, 뒤로/앞으로, 새 탭 열기 및 원문 보존이 통과했다.
- 공유 `WorkspaceModel` 기반 탭 닫기와 활성 탭 복귀도 브라우저 회귀로 검증했다.
- Files pane의 새 노트 버튼은 GPUI Workspace 이벤트를 Drive host로 넘겨 root 폴더에 Markdown을 생성하고, 반환된 Drive ID로 공유 `VaultData`, FileTree, 검색 인덱스를 갱신해 새 문서를 연다. Mocked Drive 통합에서 생성 요청과 `# 제목` 원문을 검증했다. 현재 생성 위치는 Vault root다.
- Files pane은 Desktop과 같은 `tft_search` query matcher/snippet builder 및 `tft_search::inputs` metadata 추출을 사용한다. Markdown 본문/경로, tag, frontmatter property, section/task scopes를 검색하고 결과를 탭으로 연다. 검색 직전 현재 Editor buffer를 Vault와 metadata index에 반영한다. ArrowUp/Down으로 결과를 선택하고 Enter로 여는 동작도 TextInput 이벤트를 통해 연결했다. Playwright에서 tag 검색, 다른 노트 검색, 편집 직후 본문 검색과 결과 키보드 탐색을 검증했다.
- Chrome/Playwright에서 공유 셸 렌더링, FileTree 선택 후 탭 열기, 탭 전환 중 각 편집 버퍼 보존과 편집 원문의 `VaultData` 반영을 확인했다. Editor 입력 회귀도 계속 통과한다.
- Web 좌측 dock은 Desktop과 같은 Files 단일 트리이고, Search는 메인 영역 탭으로 연다. Files 툴바와 dock 탭, 문서 헤더의 분할 아이콘은 제거해 Desktop shell과 정보 구조를 맞췄다. Drive 저장 버튼은 Web host 고유의 저장 경계로만 남긴다.
- 기존 Google Drive 브라우저 어댑터를 `crates/drive-web`으로 분리해 GPUI Web과 기존 셸이 공유한다. 일반 GPUI Web 빌드는 설정 입력, 로그인, 폴더 선택, Drive `VaultData` 구성과 Markdown 원문 로드를 거쳐 공통 Workspace를 연다. 활성 노트를 저장하는 `Save to Drive` 동작도 연결했다.
- Drive Workspace는 Cmd+S/macOS와 Ctrl+S/Windows·Linux에서 현재 Editor 원문을 저장한다. Mocked Drive 브라우저 회귀는 단축키와 `Save to Drive` 버튼의 PATCH 본문을 각각 확인한다.
- Google 로그인 토큰은 브라우저 저장소에 만료 시각과 함께 보관하고, reload 때 GIS 로드보다 먼저 복원한다. 유효한 토큰이 있으면 GIS가 늦게 로드되거나 실패해도 마지막 Vault를 다시 열며, 만료 시에는 GIS의 silent(`prompt: none`) 갱신을 시도한다. GIS token client는 refresh token을 노출하지 않으므로, Google 세션도 만료된 뒤의 완전한 재로그인 없는 복원은 서버 측 token broker 없이는 보장하지 않는다.
- OAuth scope는 `drive`다. Picker로 고른 폴더 안의 기존 노트 쓰기·휴지통 이동은 `drive.file`이 반환 항목별 권한만 주기 때문에 동작하지 않는다. scope 문자열이 바뀌면 저장된 토큰은 폐기되고 재동의가 필요하며, Google Cloud Console → OAuth consent screen → Data Access에 `.../auth/drive`가 등록되어 있어야 한다.
- Production Editor의 수정 이벤트를 Drive host로 전달해 Desktop과 같은 2초 debounce 자동 저장을 한다. Drive ETag와 per-note 저장 대기열을 사용해 저장 요청을 직렬화하고, Vault 교체 뒤 이전 요청 결과를 무시한다. Mocked Drive 브라우저 회귀에서 원문 PATCH를 확인했다.
- Drive Vault는 root 메타데이터만 먼저 읽고, 폴더를 펼칠 때 그 폴더의 자식만, 노트를 열 때 본문만 가져온다. 본문이 오기 전에는 빈 Editor 대신 공유 `workspace_loading_notice`를 그린다. `loaded_texts`가 없는 노트는 `current_document`/`store_*_editor_text`/`refresh_reading`/자동 저장 구독에서 모두 제외하므로 로딩 중 빈 문자열이 Drive로 업로드되지 않는다. Mocked Drive 회귀에서 본문 요청을 붙잡아 로딩 상태와 PATCH 0건을 확인했다.
- 한 노트가 여러 split 그룹에 열려 있으면 본문 도착 이벤트가 모든 그룹의 Editor/ReadingView에 전달된다. 분할로 새로 드러난 탭도 `sync_group_document`에서 본문을 요청한다.
- Reading으로 전환할 때 ReadingView가 키보드 포커스를 받는다. 이전에는 unmounted Editor가 포커스를 쥐어 Cmd/Ctrl+\\ 같은 Workspace action이 dispatch되지 않았다. Playwright에서 reload 후 편집→Reading 전환 뒤 dock toggle을 확인했다.
- 탭을 다른 그룹이나 pane 가장자리로 드래그하면 공유 `WorkspaceDropZone`/`WorkspaceModel::split_with_tab`으로 분할한다. 탭이 하나뿐인 그룹을 자기 자신 옆으로 분할하는 것은 공유 모델이 허용하지 않으며 Desktop과 동일하다.
- 저장되지 않은 문서가 남아 있으면 `Switch Vault`는 저장을 즉시 시작하고 성공할 때까지 Workspace를 유지한다. Mock Drive 흐름에서 debounce 전에 전환을 눌러도 최신 Markdown PATCH 후에만 Vault를 닫는 것을 확인했다.
- Chrome에서 GPUI Drive 시작 화면, 입력 컴포넌트 동작, OAuth/Picker 설정의 browser storage 저장 및 reload 유지, 콘솔 오류가 없음을 확인했다. OAuth 실제 승인, Picker 응답, Drive 파일 읽기/쓰기 왕복은 유효한 Google Cloud 설정과 실제 Vault가 없어 아직 검증하지 않았다.
- 브라우저 네트워크 계층에서 GIS/Picker/Drive를 mock한 `test_drive_flow.py`로 토큰 콜백부터 폴더 메타데이터·파일 목록·Markdown 원문 GET, 편집, 업로드 PATCH의 정확한 원문 본문까지 확인했다. 이는 Google 서버와 실제 계정의 승인을 검증한 테스트가 아니다.
- `browser-test` feature는 Drive 네트워크 호출 대신 편집 회귀 fixture를 사용한다. fixture에서 FileTree 선택, 탭 상태, 버퍼 저장, 입력/선택/클립보드/Undo/Redo가 검증된다.
- Desktop의 실제 `Workspace` 전체(도크, 검색, 설정, 뷰 타입, 그룹/레이아웃)는 아직 GPUI Web에 이식되지 않았다. 현재 Web Workspace는 `WorkspaceModel`과 `VaultData`를 쓰는 제한된 GPUI 구성이다.
- 기존 Leptos 앱은 `apps/web-leptos`에 보존했고, GPUI Web 앱은 `apps/web`에 둔다.

## 검증 및 제약

- Native: `cargo check -p desktop`
- WASM: `apps/web/check-wasm.sh`
- Browser Drive shell: `apps/web/build-web.sh`와 `tests/test_drive_shell.py`
- Mocked Drive round-trip: `apps/web/build-web.sh --features drive-smoke-test`와 `tests/test_drive_flow.py`
- Browser editor regression: `apps/web/build-web.sh --features browser-test`와 `tests/test_browser.py`
- `RUSTC_BOOTSTRAP=1`은 pinned GPUI 의존성이 `wasm_thread`의 nightly feature를 사용하기 때문이다.
- GPUI revision `4ce18741ad2ea007cb20a1dd96d2a170d224cc01`의 Web `Application::run`은 비동기 시작 이후에도 callback을 유지해야 한다. 현재 WASM 전용 패치와 적용 스크립트가 `apps/web/patches/`에 있다.
- 한글 입력 회귀는 합성 composition event 수준이며, 실제 OS IME와 iOS PWA는 검증하지 않았다.

## 완료 기준

- 브라우저에서 GPUI Web 기반 TFT 화면이 실행되고, 공통 Desktop UI/앱 동작을 실제 사용자 흐름에 사용한다.
- Desktop과 Web은 독립된 문서 편집 모델이나 별도 Workspace 상태를 만들지 않는다.
- Drive 문서의 원문 보존, 저장/재열기, 오류 없는 핵심 브라우저 흐름을 확인한다.
- iOS PWA 지원은 데스크톱 브라우저 성공 기준과 별도로 실기기에서 승인한다.
