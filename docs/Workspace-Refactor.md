---
aliases:
  - Workspace Refactor
tags:
  - tft/development
---
# Desktop UI 웹 포팅 계획

## 목표

Web 화면을 Desktop workspace와 같은 정보 구조와 조작 흐름으로 포팅한다. 공통 UI와 플랫폼 중립 동작은 `crates/app`에 구현해 Desktop과 Web이 함께 사용한다. Desktop은 창·파일시스템·OS 효과를, Web은 브라우저·Drive 인증과 저장을 각 앱에서 제공한다.

현재 Web은 공유 `Editor`, `ReadingView`, `WorkspaceModel`, 일부 `app::ui` 위젯을 사용하지만, workspace 외곽과 화면 조합을 별도로 구현한다. 그 결과 빈 탭, 탭 헤더, 사이드바, 분할 및 상태 표시가 Desktop과 다르거나 빠져 있다. 화면 스크린샷에서 빈 탭이 편집 커서만 보이는 것이 우선 확인된 차이다.

## 경계 원칙

- **공통 동작과 UI는 `crates/app`에 모읍니다.** Desktop과 Web이 같은 의미로 쓰는 상태 전이, 화면 조각, command와 사용자 동작은 공유 경계에 두고, 각 host는 플랫폼 차이와 callback/event 연결만 제공합니다. 같은 UI나 전이를 호스트별로 복사하지 않습니다.
- `crates/app`: 두 호스트가 같은 의미로 제공하는 workspace UI 조각, 화면 모델, 사용자 동작 전이, 앱 서비스.
- `apps/desktop`: native window/frame, OS 단축키·파일 선택, 로컬 Vault watcher와 파일 I/O, Desktop 전용 패널 및 효과.
- `apps/web`: 브라우저 부트스트랩, Drive 로그인·권한·업로드, URL 및 브라우저 API 어댑터.
- 공통 컴포넌트는 호스트별 callback/event를 입력으로 받는다. UI를 복사해 두 벌로 유지하지 않는다.
- `crates/domain`에는 GPUI나 화면 표현을 넣지 않는다. 앱 UI는 `crates/app`의 `gpui-ui` 경계 안에 둔다.
- 현재 앱의 기능 차이를 숨기지 않는다. Web에서 동작하지 않는 명령은 비활성 또는 미노출하고, 빈 동작 버튼을 만들지 않는다.
- Vault 파일은 사용자가 편집한 경우에만 바꾸며 줄 끝, BOM, 미지 JSON/YAML 키를 보존한다. 기존 Markdown parser 하나를 계속 사용한다.

## 포팅 순서

### 0. 공통 UI 기반과 빈 workspace — 진행 중

- [x] 기존 Desktop/Web 화면, 공유 컴포넌트와 앱 효과의 경계를 조사한다.
- [x] 이 문서를 UI 포팅 목표와 순서로 교체한다.
- [x] 빈 탭의 중앙 액션 목록을 `crates/app::ui` 공통 위젯으로 만들고 Desktop/Web에서 사용한다.
- [x] 빈 `Tab` 모델을 빈 화면으로 판별하고 Web의 새 노트/캔버스/파일 찾기 액션을 연결한다.
- [x] Web 빈 탭에서 지원하는 새 노트/파일 찾기 단축키 힌트를 표시하고 Cmd/Ctrl+N, Cmd/Ctrl+P 실제 바인딩을 제공한다.
- [x] 빈 탭 액션 위젯의 높이·색상·hover·키 힌트를 공통 `app::ui` 컴포넌트로 맞춘다.

### 1. Workspace shell과 탐색

- [x] 좌측 dock의 공통 base geometry와 배경을 `app::ui`로 옮기고 양쪽 호스트에서 사용한다.
- [x] Desktop과 Web에서 dock header, panel, footer, resize handle의 세로 shell 조립을 공유한다.
- [x] 하단 dock footer의 높이·구분선·Vault profile 버튼 레이아웃을 `app::ui`로 공유한다.
- [x] dock resize handle의 geometry, hover/drag accent를 `app::ui`에서 공유하고 Web 폭을 저장/복원한다.
- [x] 좁은 viewport에서 dock이 본문을 밀어내지 않도록 `app::ui::workspace_dock_width_for_viewport`가 저장 폭을 main 최소 폭(360px) 안으로 제한하고, 510px 미만에서는 dock을 숨긴다.
- [x] 좌측 dock 상단 header row의 공통 높이·배경·구분선을 양쪽 호스트에서 공유한다.
- [x] Web dock 접기와 탭 strip 복원 진입점을 연결한다.
- [x] Web에서 Cmd/Ctrl+\\로 좌측 파일 dock을 토글하고 상태를 저장한다.
- [x] 좌측 dock의 shell·폭 제한 규칙·footer/profile geometry를 공통 UI로 옮긴다.
- [x] `WorkspaceDockState`의 표시/선택 전이 API를 Desktop과 Web이 함께 사용한다.
- [x] Files 툴바 frame과 정렬·현재 파일 표시·전체 펼침·새 노트 진입점을 Desktop/Web 양쪽에서 제거했다. 두 호스트의 좌측 dock은 Files 트리 하나만 두고, Search는 Desktop과 같이 메인 영역 탭으로 연다.
- [x] Files sort menu의 선택 행과 구분선, 옵션 순서를 `app::ui`에서 공유한다.
- [x] Web Files 정렬 설정을 `tftWeb` workspace 상태에 저장·복원한다.
- [x] 파일 트리 표시 설정과 Search 상호작용을 대조했다. Desktop의 unsupported-file 설정은 현재 공통 상수로 고정되어 있으며 Indexed `VaultData` 경로도 동일하게 숨긴다. Search의 공통 엔진, 매치 하이라이트, 결과 이동·접기·컨텍스트 메뉴 동작은 Web에 연결되어 있다.
- [x] `.tft/app.json`의 제외 파일 규칙을 Web Search와 Graph에 적용하고, `.tft/switcher.json`의 첨부파일 표시·제외 결과 우선순위를 Web 빠른 파일 전환기에 적용한다. 빠른 전환기의 파일 종류 필터, 파일명/경로/alias 점수, 최근 파일 목록, 제외 순위와 tie-break를 `crates/app::file_switcher`에서 두 호스트가 공유한다.
- [x] Web Search 결과에 공통 검색 엔진의 이름/본문 매치 하이라이트, 결과 수, 6종 정렬, 전체 접기/펼치기를 연결한다.
- [x] Search 결과의 6종 정렬 comparator를 `crates/app::search_results`로 공유해 Web도 Desktop과 같은 자연 정렬, 이름 동률 시 역순 전체 경로, 시간 정렬의 stable tie를 따른다.
- [x] Desktop Search의 Match case 동작을 `tft_search::Query::CaseSensitive`로 연결하고 공통 Search shell에 옵션 컨트롤 슬롯을 둔다.
- [x] Search 설정에서 전체 결과 접기, 추가 문맥, 검색식 설명을 토글하고 같은 `tft_search` query tree를 표시한다.
- [x] Search 결과의 Up/Down이 파일/일치 문장 행을 순회하고, Enter 이동·Escape 선택 해제·Left/Right 접기/펼치기를 Desktop과 맞춘다.
- [x] Search 결과에서 파일별 접기/펼치기, 본문 매치 클릭 시 해당 위치로 caret 이동, 우클릭 파일 메뉴를 연결한다.
- [x] 파일 행의 shell, chevron, 들여쓰기 가이드, 긴 이름 말줄임과 확장자 태그를 공유한다.
- [x] Indexed `VaultData`에서 만드는 Web 파일 트리도 Desktop과 같이 unsupported 파일을 기본 숨김으로 처리하고, `FileTree::from_vault_with_options`에서 동일 옵션을 받는다.
- [x] 파일 트리에서 Ctrl/Command 클릭으로 새 탭을 여는 동작을 Desktop과 맞춘다.
- [x] Web 파일 트리에 포커스 가능한 키보드 탐색(위/아래, 좌/우, Enter, Space, Escape, Tab)을 연결한다.
- [x] Up/Down 시작 행과 Left/Right 폴더 전이를 `FileTree`에서 공유하고 Desktop/Web이 함께 사용한다.
- [x] Web 파일 트리에 지원되는 우클릭 메뉴 동작(새 노트, 새 탭 열기, 탐색기에서 표시, 경로 복사, 폴더 펼치기/접기)을 연결한다.
- [x] Files tree context menu의 항목 순서·섹션·파일/폴더별 활성 조건을 `crates/app::file_tree_menu`에 두고 Desktop/Web이 함께 사용한다. Web의 Rename은 공용 Prompt와 Drive 링크 갱신, Duplicate는 Drive copy와 공용 Vault/Search 인덱싱, Delete는 확인 후 Drive 휴지통 이동과 dirty-save 보호 및 workspace/index 정리에 연결한다.
- [x] Web에서 폴더/파일의 우클릭 메뉴로 해당 Drive 폴더 안에 Markdown 노트를 생성하고, 새 노트를 즉시 트리에 반영한다.
- [x] Web 파일 트리 우클릭 메뉴에서 하위 Drive 폴더를 생성하고, 새 폴더를 즉시 트리에 반영한다.
- [x] Web 파일 트리 우클릭 메뉴에서 해당 Drive 폴더 안에 `.canvas`를 만들고 즉시 Canvas 탭으로 연다.
- [x] Web 파일 우클릭 메뉴에서 “Open to the right”로 새 pane을 만들고 해당 파일을 연다.
- [x] Web 파일 우클릭 메뉴에 OS 앱 열기의 Web 대응인 “Open in Google Drive”를 제공한다.
- [x] Web 파일 트리 우클릭 메뉴에서 파일의 상위 폴더 또는 폴더 자체를 범위로 하는 Search 탭을 열고, `path:"folder/"` query를 입력한다.
- [x] Desktop의 native frame/OS 효과와 Web의 브라우저/Drive 제어는 각 host에 둔다.

### 2. 탭과 문서 헤더

- [x] 탭/헤더 제목의 공통 표시 규칙을 `crates/app`으로 옮기고 Desktop/Web에서 사용한다.
- [x] 탭 strip의 외부 프레임 높이, 배경, clipping과 하단 hairline을 공통화한다.
- [x] 새 탭 추가 전이를 `crates/app`에 두고 Desktop/Web의 `+` 버튼에서 사용한다.
- [x] 탭 내부 여백, 비활성 hover, hover 시에만 보이는 닫기 아이콘을 양쪽 앱에서 공유한다.
- [x] 탭 제목 말줄임과 닫기 슬롯 배치를 공통 `workspace_tab_content`로 렌더링한다.
- [x] 탭 strip의 가로 스크롤 viewport를 양쪽 앱에서 공유한다.
- [x] 탭 strip의 뒤로/앞으로 컨트롤 셀과 탭 사이 구분선을 양쪽 앱에서 공유한다.
- [x] 뒤로/앞으로 컨트롤과 탭 strip 구분선의 조립을 `workspace_tab_strip_navigation`으로 공유한다.
- [x] 비활성 탭의 내부 divider, 새 탭 버튼, 핀 탭의 unpin 버튼을 양쪽에서 같은 공통 컴포넌트로 사용한다.
- [x] 폴더 breadcrumb 데이터와 표시 스타일을 공유하고 Web에서 클릭해 탐색기를 해당 폴더로 이동한다.
- [x] 모드 전환 버튼 표시 조건을 공유해 빈 탭과 지원하지 않는 view에 숨긴다.
- [x] Web에서 중간 클릭으로 탭을 닫을 수 있게 한다.
- [x] Web 탭 우클릭 메뉴에 닫기, 다른 탭 닫기, 뒤쪽/전체 탭 닫기, pin, 복제, 탐색기에서 표시, 상대 경로 복사를 연결한다.
- [x] Web 탭 우클릭 메뉴에 활성 문서의 Split right/down과 Google Drive 열기를 연결한다.
- [x] Web 활성 파일 탭 메뉴에 Desktop과 같은 Rename, Move file to, Copy path, Reveal in navigation, Delete file 동작을 Drive host 흐름으로 연결한다.
- [x] workspace pane 최대 개수와 split 가능 여부를 `MAX_WORKSPACE_GROUPS` / `workspace_can_split`으로 공유하고, Web의 각 pane header가 최대치에서 Split 도구를 숨긴다.
- [x] Web 활성 파일 탭 메뉴에 이름 편집을 연결하고, 편집 중인 Markdown 탭에서는 Find, Replace, Add file property를 제공한다.
- [x] split pane의 탭 우클릭 메뉴가 활성 pane이 아니라 실제 우클릭한 group ID와 탭 인덱스를 대상으로 동작하게 한다.
- [x] split pane의 Drive 저장 버튼을 누르면 해당 group을 활성화한 뒤 저장 event를 보내 그 pane의 Markdown 문서를 저장한다.
- [x] Desktop/Web 문서 헤더의 하단 구분선, breadcrumb/title lane, tools lane 치수를 `app::ui` 공통 프리미티브로 맞춘다.
- [x] 문서 헤더의 공통 lane 조립을 `workspace_document_header`로 통합하고, 각 host는 제목과 도구만 제공한다.
- [x] tab viewport, tab 목록, 새 탭 버튼과 host trailing slot 조립을 `workspace_tab_strip_tabs`로 공유한다.
- [x] 탭 선택·닫기·pin 전이를 공통 workspace API로 옮긴다. Web은 `WorkspaceModel` API를, Desktop은 동일한 공통 tab-index 전이를 사용한다.
- [x] 탭 드래그 source의 group ID와 tab index를 `TabDragSource`로 공유한다. 각 host는 drag preview와 화면 좌표 hit-testing을 유지한다.
- [x] 다음/이전 탭의 순환 index 전이를 `crates/app::cycle_tab_index`로 공유하고 Desktop/Web command dispatcher에서 사용한다.
- [x] 뒤로/앞으로 history 전이, breadcrumb/title 규칙과 mode switch 표시 조건을 `crates/app`에서 공유하고 호스트별 UI action만 연결한다.
- [x] 활성 history entry의 mode/caret 위치 기록을 `update_history_entry_state`로 공유하고, 현재 노트 path가 바뀐 stale update는 무시한다.
- [x] Reading의 top block, editing의 selection start, Graph 이전 문서 모드 분기를 `document_history_state`로 공유해 Desktop/Web이 같은 history offset을 저장한다.
- [x] Web history/persistence offset을 활성 view에서 수집한다. Reading은 reader의 top block, editing은 Editor selection 시작점을 사용하고 Graph는 직전 문서 모드의 위치를 보존한다.
- [x] Web에서 pane 간 탭 드래그를 source 제거·target 삽입 공통 전이로 처리하고, source가 비면 pane과 view subscription을 정리한다.
- [x] Web의 Reading/Editor 내부 Markdown 링크에 설정 기반 Cmd/Ctrl page preview를 연결한다. 공용 ReadingView로 렌더링하고 300 ms hover 지연, window anchor, heading/block subpath 위치를 적용한다.
- [x] Web page preview 안의 ReadingView 링크도 preview 문서 경로를 기준으로 해석한다. 클릭은 일반 링크 전이를 실행하고, hover는 부모 preview를 유지한 채 최대 8단계 중첩 preview를 표시한다. 본문 클릭은 공유 Editor로 전환하고 caret을 클릭 위치에 둔다.
- [x] Desktop view header의 인라인 파일명 편집을 Web에 포팅하고, Drive rename·Vault 경로 갱신·링크 업데이트 확인 흐름까지 연결한다.
- [ ] 드래그, 우클릭 메뉴, 창별 탭 persistence 중 호스트 의존 부분을 분리한다.
- [x] 탭 드래그 hit test의 삽입 경계 계산을 `crates/app::workspace::nearest_tab_drop_boundary`로 공유한다. Desktop은 strip의 측정된 모든 tab 경계에서, Web은 hover된 tab의 양쪽 경계에서 같은 계산기를 사용한다. 픽셀 측정과 Desktop의 window/popout lifecycle은 host에 남긴다.
- [x] 탭 메뉴의 Close others/Close tabs after/Close all 행 가용성과 pinned-tab 보존 규칙을 `crates/app::workspace::{tab_close_action_available, tab_close_action_keeps}`로 공유한다. Desktop menu/command 실행과 Web context menu가 같은 규칙을 쓴다.

### 3. 문서 화면

- [x] 같은 Editor와 ReadingView 구현을 두 호스트에서 사용한다.
- [x] Web에서 workspace 문서를 여는 공통 Editor API가 저장된 `LivePreview`/`Source` 모드를 적용해 복원과 탭 전환 시 렌더링 모드가 어긋나지 않게 한다.
- [x] 새 Markdown 문서, 빈 탭 및 복원 모드 fallback을 `ViewMode::markdown_document_mode`로 공유해 Desktop/Web이 같은 Live Preview 기본값을 사용한다.
- [x] 공유 `ViewMode::parse`가 `terminal`도 복원하도록 하고, Web은 Desktop Terminal 탭을 Source 편집기로 잘못 열지 않고 지원 불가 안내 화면으로 보존한다.
- [x] Web의 Reading↔LivePreview 전환이 Desktop과 같이 편집 버퍼를 보존하고, caret/history offset·fold 상태를 넘기며, 읽기 화면으로 나갈 때 Drive 저장을 즉시 요청한다.
- [x] Live Preview↔Source 순수 모드 전이를 `ViewMode::toggle_source_preview`로 공유하고 Desktop/Web이 같은 전이를 쓴다.
- [x] wikilink target 파싱과 vault-relative destination resolution을 `crates/app::resolve_internal_link`으로 공유해 Desktop과 Web이 같은 LinkResolver 규칙을 쓴다.
- [x] 내부·외부·태그·footnote 링크 분류와 내부 링크 해석을 `crates/app::resolve_link_navigation`으로 공유한다. Web은 태그를 공용 Search로 보내고 footnote 목적지로 편집기/읽기 위치를 이동한다.
- [x] 링크로 노트를 열 때 Terminal pane 유지, pinned tab 보존 및 명시적 새 탭 요청을 `linked_note_opens_new_tab`으로 공유해 Web과 Desktop의 탭 결정 규칙을 맞춘다.
- [x] Web의 링크/파일 열기 새 탭은 Desktop과 같은 기본 편집 모드로 시작하며, Graph에서 현재/다른 노트를 열 때 직전 문서 모드를 복원한다.
- [x] Web의 누락된 Markdown/Canvas wikilink를 무시하지 않고 종류에 맞는 Drive 생성 Prompt를 링크 이름으로 미리 채운다. `.tft/app.json` 새 노트 위치를 기본으로 쓰며, 링크에 존재하는 하위 폴더가 있으면 그 폴더를 부모로 쓴다.
- [ ] Preview와 문서별 도구 동작을 두 호스트에서 맞춘다.
- [ ] 링크 이동, 선택/커서, 편집/저장 전이가 공유 앱 API를 통하도록 정리한다.
- [x] Web Editor의 해석 가능한 wikilink context menu에서 파일명 편집, Move file to, 경로 복사, Google Drive 열기와 Files 탐색기에서 표시를 Drive host 동작으로 연결한다. Move는 Desktop과 같은 대상 폴더 제안과 링크 갱신 확인을 제공하고, Drive parent 변경·Vault 경로·history·열린 pane·검색·Graph를 갱신한다.
- [x] Web ReadingView의 유효한 내부 링크 우클릭 메뉴를 Editor와 같은 파일 동작(Open in new tab/right, Copy path, Rename, Move, Google Drive 열기, 탐색기에서 표시)으로 연결한다. 편집 전용 링크 삽입·clipboard 메뉴는 ReadingView에서 숨긴다.
- [x] Canvas edge label 추가/편집 UI를 포팅한다. 선택한 edge의 Add/Edit label 액션이 host의 공용 Prompt를 열고, 저장한 문자열을 공용 `Scene` transaction으로 반영해 Drive autosave한다. Web Canvas는 공용 `Scene`과 minimap geometry를 쓰며 카드/edge 표시, 휠/버튼 확대·축소와 fit 초기화, 배경 드래그 pan, edge 선택·방향 순환·label 표시와 삭제, Shift-click 다중 선택과 선택 묶음 이동, 선택 항목 그룹 생성, 선택 카드의 8방향 resize handle, text card 추가, Desktop과 같은 공유 `Editor`로 텍스트 카드 더블클릭 편집, 선택 항목의 기본색/6개 preset 변경, 선택 항목 삭제, undo/redo를 지원한다. 편집 완료와 문서 전환 때 Scene transaction을 끝내고 Drive에 저장한다. Mermaid는 `gpui-ui` feature에서 공용 `tft_mermaid`를 포함하고 Editor의 Mermaid block renderer를 사용한다. Image는 인증된 Drive blob URL로, PDF는 공용 `PdfView`/페이지 cache에 Drive 바이트를 공급해, Audio는 브라우저 `Audio` player로 탭 안에서 렌더링한다. Video는 인증된 Drive blob URL을 브라우저 native `<video controls>` overlay에서 재생한다.
- [x] Graph의 vault node/edge builder와 force layout을 `crates/app`에서 Desktop/Web이 공유한다.
- [x] Web Graph 기본 canvas, 자동 fit, force simulation, pan/zoom 및 Markdown 노트 열기를 연결한다.
- [x] Desktop `graph.json` 읽기/저장, node hover와 local graph 동작을 확인하고 Web의 공유 builder/layout과 맞춘다.
- [x] GraphConfig의 전체/부분 JSON serializer를 `crates/formats`에서 공유하고 Web Drive도 `.tft/graph.json`을 읽고 쓴다. `tftWeb.graphConfig`는 이전 Web 설정 및 저장 실패의 recovery fallback으로만 남긴다.
- [x] Web Graph panel의 Tags, Attachments, Existing files only, Orphans, Arrows 토글, Display/Forces sliders, color groups를 builder/layout과 연결하고 `tftWeb.graphConfig`로 workspace 저장한다.
- [x] Web Graph node hover highlight/label과 Local graph 중심 노트, Depth 및 incoming/outgoing/neighbor 링크 옵션을 workspace 저장에 연결한다.
- [x] Web의 단일 pane과 split pane header에서 공유 force Graph와 `radial-tree-explorer`를 선택하고, 현재 graph 종류를 활성 표시한다. radial-tree 노트 선택은 해당 pane의 문서 탭으로 연다. 폴더/Markdown 트리와 이벤트 경로 인덱스 생성은 Desktop/Web이 `app::explorer::radial_tree_data`를 공유한다.

### 4. 분할, 상태 및 보조 UI

- [x] 공유 `Layout`의 split/group 삽입·제거·크기 조절과 두 tab strip 사이의 이동 전이를 `crates/app`에서 제공한다. Web은 이를 `WorkspaceModel`로 관리하고, Desktop은 multi-window/popout lifecycle만 host에서 관리한다.
- [x] split divider의 포인터 좌표 정규화(`normalized_split_position`), 최소 pane 크기 제한과 인접 pane 비율 재분배(`split_dimensions_at`)를 공유한다.
- [x] Desktop과 Web의 split row/column 및 pane separator 기본 geometry를 `crates/app::ui`로 공유한다.
- [x] Web 헤더에서 오른쪽/아래 split을 만들고 현재 탭을 새 그룹에 복제하며, pane을 눌러 그룹을 활성화하고 활성 pane을 닫는다.
- [x] Web split 시 새 그룹 생성·활성화·현재 탭 복제를 `WorkspaceModel::split_with_active_tab` 전이로 처리하고, host는 새 그룹의 Editor/ReadingView 생성만 맡는다.
- [x] Web divider의 정규화된 위치를 `WorkspaceModel::resize_split_at`에 전달해 layout 차원 변경을 모델 전이로 처리한다. GPUI 포인터 좌표만 host가 계산한다.
- [x] Web pane 활성화 전이를 `WorkspaceModel::activate_group`으로 통일해 모델과 split layout에 모두 존재하는 그룹만 활성화한다.
- [x] pane 간 탭 이동의 source 제거와 target 삽입을 `transfer_tab_between_strips`로 공유한다. Web의 `WorkspaceModel::transfer_tab`은 마지막 탭 이동 시 source 그룹/layout 제거와 target 활성화를 처리하고, Desktop은 popout/window 규칙을 적용한다. 각 host는 live view를 동기화한다.
- [x] Web은 `WorkspaceModel.layout`의 row/column 트리를 따라 그룹별 Editor/ReadingView 내용을 분할 렌더링한다.
- [x] split 상태에서 그룹별 tab strip과 document header를 각 pane 안에 렌더링한다.
- [x] Web에서 split divider를 드래그해 pane 비율을 바꾸고 workspace 저장/복원에 포함한다.
- [x] 검색/명령 대화상자, 메뉴 frame/item, prompt, hover preview를 공통 위젯으로 맞춘다. 메뉴와 파일별 action은 각 host callback으로 연결한다.
- [x] 명령 팔레트 위젯을 `crates/app`으로 옮겨 Desktop/Web에서 공유하고, Web `Cmd/Ctrl+Shift+P` 및 지원 command 실행을 연결한다.
- [x] 닫힌 탭의 20개 보관·복원 순서를 `crates/app::ClosedTabHistory`로 공유하고 Web command palette의 `workspace:undo-close-pane`에 연결한다. Web에서는 브라우저의 `Cmd/Ctrl+Shift+T`와 충돌하지 않도록 해당 단축키를 바인딩하지 않는다.
- [x] Web이 탭·pane 전환, split, 링크/history 이동, 닫기 전에 활성 뷰의 history offset을 기록하고 다시 활성화할 때 Editor caret 또는 Reading 위치를 복원한다.
- [x] hover row의 window-space anchor 측정을 공통 `app::ui::hover_row`로 옮기고, Web Files의 Cmd/Ctrl hover에서 공용 `ReadingView` Markdown preview를 표시한다.
- [x] 공통 Prompt 컴포넌트를 `crates/app`에서 Desktop/Web이 사용하고, Web Drive의 새 노트/폴더 입력도 앱 내부 모달로 받는다.
- [x] status bar는 Desktop에서 화면에 렌더되지 않고 control API용 값만 제공함을 확인했다. Web에 별도 시각 UI를 추가하지 않는다.
- [x] Web의 파일 선택 진입점에 공통 팔레트 UI, 이름/경로/Markdown alias 검색, 최근 파일, Cmd/Ctrl+P, 위/아래·Enter·Escape를 연결하고 Desktop과 같은 공유 ranking을 사용한다. 결과는 Markdown·Canvas·PDF·미디어 파일을 포함한다. 모든 파일은 workspace 탭으로 열고 새로고침 후 복원한다. Image는 내부 렌더링하고 현재 viewer가 없는 비-Markdown 형식은 명시적 Google Drive 미리보기 액션을 제공한다.
- [x] Search, quick switcher의 위/아래 선택 인덱스 전이를 `app::selection_step`으로 공유한다. Desktop Search와 Web Search는 경계에서 멈추고 첫 항목 위에서 선택을 해제한다. Quick switcher는 순환 선택을 유지한다.
- [x] Web의 파일 찾기·새 노트·dock 토글 단축키는 Desktop과 같은 command ID를 dispatch한다.
- [x] Web에서 Desktop의 `app:go-back`/`app:go-forward` 단축키를 같은 history 전이에 연결한다.
- [x] Web의 `global-search:open` command palette/shortcut을 Desktop처럼 중앙 Search tab에 연결하고 기존 query를 유지한다.
- [x] Web에서 Desktop의 `markdown:toggle-preview` command를 Reading↔LivePreview 공통 모드 전환에 연결하고 caret offset을 유지하며, Terminal pane에서는 전환하지 않는다.
- [x] Web에서 Desktop의 `graph:open` 단축키를 현재 pane의 공유 Graph 전환에 연결한다.
- [x] Web의 Cmd/Ctrl+S를 공통 `editor:save-file` command로 dispatch하고, Drive 저장은 host event로 처리한다.
- [x] Wikilink의 `#Heading`/`#^block` subpath에서 metadata offset을 찾는 처리를 `crates/app::subpath_offset`으로 공유하고, Desktop/Web 편집기·읽기 화면을 해당 위치로 이동한다. 기존 Markdown parser를 사용한다.
- [x] Web에서 지원하는 명령의 기본 단축키를 공유 `tft_commands::default_hotkeys`에서 등록한다. 브라우저 OS를 빌드 타깃으로 알 수 없으므로 `Mod`는 Cmd와 Ctrl 바인딩으로 확장하고, Web이 처리하지 않는 기본 명령은 브라우저 단축키를 가로채지 않도록 제외한다.
- [x] Web 명령 팔레트의 `Mod` 힌트는 WASM target OS 대신 browser runtime platform을 사용해 Mac에서 ⌘, 다른 플랫폼에서 Ctrl로 표시한다.
- [x] Web Vault의 `.tft/hotkeys.json` 재정의를 읽어 command palette와 실제 키 바인딩에 반영한다. 재정의된 명령은 이전 기본 바인딩을 무시하고 사용자 지정 키만 실행한다.

### 5. 통합 및 parity 정리

- [ ] 동일한 Vault fixture와 동일한 workspace 상태에서 Desktop/Web 화면을 비교한다.
- [ ] 각 단계의 사용자 조작을 양쪽에서 확인하고 미지원 동작은 명시한다.
- [ ] Specification 및 관찰 동작과 어긋나는 부분은 probe/fixture 근거를 추가해 갱신한다.
- [x] 빌드·검증 결과를 기록한다. 요청되지 않은 테스트는 실행하지 않는다. Web WASM (`RUSTC_BOOTSTRAP=1 cargo check --manifest-path apps/web/Cargo.toml --target wasm32-unknown-unknown --features drive-smoke-test --jobs 1`), Desktop (`cargo check -p desktop`), `cargo fmt --all -- --check`, `git diff --check`가 통과했다.

### Web 저장과 재시작 복원

- [x] 마지막 Drive Vault를 기억하고, 재방문 시 Google GIS의 무화면 access token 재개를 시도한다.
- [x] 이전에 연 Vault가 없어도 저장된 Google 설정으로 GIS 세션을 먼저 복구해, 새로고침 뒤 다시 로그인하지 않고 Drive 폴더를 선택할 수 있게 한다.
- [x] 새로고침·새 탭에서 만료 전 Google access token을 브라우저 저장소에서 복구하고 마지막 Vault를 자동으로 다시 연다.
- [ ] 만료 토큰이나 Google GIS의 무화면 재개 거부 상황에서 재인증 없이 유지한다. 정적 Web client는 OAuth refresh token을 보관하지 않으므로, 이 보장에는 서버 측 OAuth code flow와 세션이 필요하다.
- [x] Web의 탭, 활성 문서, 읽기/편집 모드, 핀, dock 표시/선택/폭, 트리 정렬·폴더 펼침, 최근 파일을 `.tft/workspace.json`의 `tftWeb` 확장 키에 저장하고 복원한다.
- [x] 그룹별 탭·활성 탭과 split 방향/트리를 저장하고 복원하며, 이전 단일 그룹 데이터도 계속 읽는다.
- [x] 탭별 back/forward history와 현재 history index, 각 history entry의 editor offset을 저장·복원한다.
- [x] Drive 저장 전 Web workspace 상태를 브라우저 로컬 임시본으로 즉시 보관하고, Drive 저장 성공 뒤 임시본을 지우며, 복원 때 남은 임시본을 Drive에 다시 반영한다.
- [x] Canvas별 확대율과 pan 위치를 Vault-relative path keyed `tftWeb.canvasViewports`에 저장하고 복원한다.
- [x] 기존 `workspace.json`의 미지 키와 BOM/CRLF를 보존하며, 파싱할 수 없는 설정 파일은 덮어쓰지 않는다.
- [ ] 브라우저 런타임에서 실제 Drive 계정으로 새로고침 복원을 확인한다.

Google 로그인 정보와 Client ID/API key, 마지막 Vault는 브라우저 설정에 남긴다. OAuth access token은 만료 시각과 OAuth Client ID를 함께 기록해 브라우저 저장소에서 복구하며, GIS가 발급한 짧은 수명의 토큰만 저장하고 refresh token은 저장하지 않는다. 앱 시작 시 마지막 Vault 유무와 관계없이 GIS 세션 복구를 시도하고, 성공하면 저장된 Vault를 열거나 Vault 선택 화면을 표시한다. 토큰이 만료되면 GIS `prompt: "none"`으로 재개를 시도한다. Google이 무화면 토큰 발급을 거부하면 다시 로그인해야 한다. Workspace는 변경 즉시 브라우저 `localStorage` 임시본에 기록하고 Drive의 `.tft/workspace.json`에도 debounce 저장한다. 새로고침 때 임시본과 Drive 저장본을 읽어 미전송 변경을 우선 복원하고 Drive에 재반영한다. Drive 저장 실패 시 임시본은 지우지 않는다. 실 Google 계정 새로고침 확인은 아직 남아 있다.

## 완료 기준

- 기본 workspace의 정보 구조와 조작 결과가 Desktop/Web에서 일치한다.
- 공통 UI와 전이는 `crates/app` 한 곳에 있고 양쪽 앱이 이를 사용한다.
- 파일 저장, 브라우저/OS API, 인증 등 플랫폼 효과는 명확한 host 경계에 남는다.
- 포팅되지 않은 기능과 검증되지 않은 동작을 완료로 표시하지 않는다.

## 현재 작업

0단계에서는 Web의 빈 tab 판정 오류를 고쳐 초기 화면이 편집기 빈 화면 대신 액션 목록을 그리게 했다. 1단계에서는 좌측 dock의 기본 geometry와 header row를 `app::ui`로 추출하고 `WorkspaceModel.left_visible`으로 접기/복원을 관리한다. 파일 트리는 chevron·indent 가이드·긴 이름 말줄임을 공유하며, `FileTree`의 키보드 이동 전이를 Web과 Desktop이 함께 사용한다. Web 우클릭 메뉴에는 현재 지원되는 새 노트, 새 탭 열기, 탐색기에서 표시, 경로 복사, 폴더 접기/펼치기를 연결했다. 2단계에서는 탭 strip 외부 프레임, 탭 제목 규칙, 빈 탭 추가 전이, 탭 내부 스타일, 가로 스크롤 영역, 히스토리 컨트롤 셀, breadcrumb 표시 및 폴더 탐색 동작을 공유한다. 3단계에서는 Desktop/Web이 동일한 Editor와 ReadingView 구현을 사용함을 확인했다. Web의 빈 탭 “Go to file”과 Cmd/Ctrl+P는 공유 팔레트 위젯을 사용하는 파일 선택창을 연다. 이름·경로·alias 매칭 및 전체 ranking(최근 파일, 제외 순위, 동점 처리 포함)은 `crates/app::file_switcher`에서 Desktop과 Web이 공유한다. Web은 Vault 내 Markdown·Canvas·PDF·미디어 파일을 찾고 모든 파일을 workspace 탭으로 연다. Web은 `.tft/workspace.json`의 `tftWeb` 아래에 그룹별 탭/활성 문서/모드/핀, 활성 그룹, split 방향/트리/크기, 좌측 dock, 트리 펼침과 최근 파일을 저장하고 복원하며 기존 단일 그룹 데이터도 읽는다. Web은 헤더의 split right/down 버튼으로 활성 탭을 복제한 그룹을 만들고, `WorkspaceModel.layout`의 row/column 트리를 따라 각 그룹의 Editor/ReadingView를 나눠 그린다. split 시 각 pane이 자체 tab strip, history controls, breadcrumb/header와 도구를 표시하며 각 pane 안의 탭은 드래그로 순서를 바꾼다. pane 선택, 탭 선택/닫기/pin, 새 탭, history, pane 닫기는 그룹 단위로 동작한다. split divider를 드래그해 pane 비율을 조절하며 layout split node의 `dimensions` 배열로 저장/복원한다. Google access token은 만료 시각과 OAuth Client ID를 포함해 `localStorage`와 `sessionStorage`에 저장한다. 새로고침이나 새 탭에서 유효한 토큰을 재사용하고, 만료한 경우에는 Google 세션으로 무화면 GIS 토큰 발급을 시도한다. OAuth refresh token은 보관하지 않는다. 자동 재개가 거절되면 재인증을 요청한다. workspace 변경은 Drive 저장 전 로컬 임시본으로 보존하고, 다음 로드 때 Drive에 반영한다. 이 동작의 실제 브라우저/Drive 계정 확인은 남아 있다. Web wasm `cargo check`는 `RUSTC_BOOTSTRAP=1`로 통과했다. 안정 채널 기본 검증은 `wasm_thread`의 `#![feature(stdarch_wasm_atomic_wait)]`가 nightly를 요구해 막혔다. 런타임 화면 비교도 아직 남아 있다.

최근 변경에서는 Desktop/Web의 문서 헤더 frame, 하단 구분선, breadcrumb/title 정렬 lane, tools lane을 `crates/app::ui`로 공유했다. Reading/editing 전환 아이콘 규칙도 같은 모듈의 `workspace_document_mode_icon`으로 공유한다. Web에서 빠져 있던 헤더 하단 divider도 공통 shell에 포함했다. Desktop과 Web `cargo check` 및 `git diff --check`가 통과했다. 실제 브라우저 화면 비교는 아직 하지 않았다.

Web Editor와 ReadingView의 링크 우클릭 이벤트를 group별 context menu로 연결했다. Editor는 선택 상태에 따라 Cut/Copy 활성화를 Desktop과 맞추고, Add link, Add external link, 선택 텍스트 검색, Paste/Paste as plain text, Select all을 공유 menu UI로 표시한다. 해석 가능한 wikilink에는 Open in new tab, Open to the right, Copy path, Rename, Move file to, Open in Google Drive, Reveal in navigation을 연결하며, 메뉴는 우클릭한 pane의 Editor entity에 동작한다. ReadingView에서는 같은 파일 동작을 제공하고 편집 전용 행은 감춘다. Move는 Desktop과 같은 대상 폴더 제안과 링크 갱신 확인을 제공하고 Drive parent 변경 및 workspace 경로/열린 pane/search/Graph를 갱신한다. Web clipboard는 브라우저 텍스트 clipboard만 사용하므로 두 Paste 행의 결과는 plain text다. Web WASM compile check, JavaScript syntax check와 diff 확인을 수행했다.

새 Markdown 문서의 기본 편집 모드와 복원된 mode fallback을 `ViewMode::markdown_document_mode`로 공유한다. Web의 새 탭·파일 열기·history 이동과 Desktop의 편집 기본값은 Live Preview를 사용하며, Graph/Search 같은 비문서 mode는 Markdown 문서 mode로 복원되지 않는다. Desktop/Web compile check, rustfmt와 `git diff --check`가 통과했다. Google 계정과 브라우저 새로고침 복원 동작 확인은 아직 남아 있다.

좌측 dock의 3 px resize handle도 공통 `app::ui` 컴포넌트로 추출했다. Web에서 드래그 폭을 150–현재 창 너비 한도 안에 제한하고, release 시 `.tft/workspace.json` 상태 저장을 예약한다.

Web은 Cmd/Ctrl+\\로 좌측 dock을 토글하고 변경 즉시 workspace 저장을 예약한다. breadcrumb와 파일 메뉴에서 탐색기를 열 때도 dock과 선택된 Files 탭 상태를 저장한다. 같은 활성 노트를 파일 선택기에서 다시 열 때의 최근 파일 순서와 파일 트리 정렬 순서도 workspace에 반영하고 복원한다. Desktop/Web에서 사용되는 command ID를 하나로 합치는 작업은 남아 있다.

Web 파일 트리 정렬 메뉴는 `app::ui` 메뉴 항목과 구분선을 사용하고, Desktop 검색의 정렬 메뉴처럼 두 항목 단위 구분과 바깥 클릭 닫기를 지원한다. 정렬 상태 변경은 workspace 저장 이벤트로 전달한다. Files tree context menu의 순서·섹션·파일/폴더 활성 규칙은 `crates/app::file_tree_menu`를 Desktop/Web이 공유한다. Web의 Drive 생성·검색·상대 경로 복사·파일 열기·탐색기 내 표시를 host callback으로 연결한다. 파일 Rename은 공용 Prompt에서 입력받아 기존 충돌 검사, Drive rename, 링크 갱신 확인 흐름을 재사용한다. Duplicate는 새 이름을 충돌 검사한 뒤 Drive `files.copy`를 호출하고 성공 결과를 Vault/Search 인덱스와 file entries에 추가한다. Markdown/Canvas 복사본을 읽을 수 없거나 Vault 인덱싱에 실패하면 새 복사본을 정리한다. Delete는 미저장/저장 중 파일을 막고 확인 후 Drive `trashed: true`를 설정하며, 성공하면 열린 탭·history·Vault·Search·Graph 상태를 정리한다. 아직 구현되지 않은 다른 Drive 작업은 비활성으로 표시한다. 우클릭 메뉴의 New note와 New folder는 공통 `FileOperationRequest`로 각 host에 전달한다. Web은 Drive 폴더 ID를 찾아 새 항목을 만들고 트리에 반영하며, Desktop은 기존 prompt와 파일시스템 흐름을 사용한다. 새 폴더 이름 검증은 Web에서도 공유 Vault 검증기를 사용한다.

탭 strip의 비활성 tab divider, 새 탭 버튼 geometry/icon hover, pinned 탭 아이콘 클릭의 unpin 전이를 Desktop/Web 공통 UI로 맞췄다. Web은 현재 pane별 tab strip에서 그룹 내 reorder와 pane 간 이동을 처리하며, 우클릭 메뉴에 닫기·pin·복제·탐색기에서 표시·상대 경로 복사를 제공한다. Markdown 탭은 Cmd/Ctrl hover 시 파일 행과 같은 300 ms 지연 미리보기를 띄운다. Web 파일 트리의 New folder를 Drive host에 연결하고, 공통 file operation을 Desktop의 기존 새 폴더 prompt 흐름에도 연결했다. Web WASM `cargo check`, Desktop `cargo check -p desktop --jobs 1`, `git diff --check`가 통과했다. 실제 브라우저 hover 및 drag 조작 비교는 남아 있다.

Web 빈 탭은 New note, New canvas, Go to file 액션을 표시한다. New note와 파일 선택기의 Cmd/Ctrl 단축키가 실제 동작에 연결되어 있다. Canvas 액션은 Drive에 이름 충돌 없는 `.canvas` 파일을 생성하고 Canvas 탭으로 연다.

하단 Vault profile footer의 공통 높이, 패딩, 간격, 구분선과 Vault 이름 버튼 geometry를 `app::ui`로 추출하고 Desktop/Web에서 사용한다. 설정 메뉴는 Desktop host 전용으로 남기며 Web의 Vault 전환 버튼도 같은 shared profile button을 사용한다.

좌측 dock의 header·panel·footer·resize handle 조립도 `workspace_dock_shell`로 공통화했다. 각 호스트는 고유한 header와 panel 내용, footer 동작만 전달한다.

탭 제목의 말줄임 상자와 닫기/핀 슬롯 배치를 `workspace_tab_content`로 추출해 Desktop과 Web 양쪽에서 사용한다. 각 호스트의 클릭 동작은 그대로 host callback으로 유지한다.

문서 헤더도 공통 `workspace_document_header`가 제목 lane과 tools lane을 같은 shell에 조립한다. breadcrumb 이동, Graph/모드 버튼 및 Drive 저장은 플랫폼별 callback으로 남긴다.

탭 strip의 뒤로/앞으로 버튼과 다음 tabs 영역 사이의 divider를 `workspace_tab_strip_navigation` 한 컴포넌트로 조립한다. Desktop/Web은 각각의 history action만 전달한다.

Web workspace 저장 상태를 다중 그룹 스키마로 확장했다. 각 그룹의 탭/활성 인덱스와 `Layout` 트리를 저장하고 로드하며, 이전 `tabs`/`activeTab` 단일 그룹 저장본도 읽는다. 로드한 그룹마다 Editor/ReadingView와 group ID별 이벤트 구독을 생성한다. 오른쪽/아래 split 버튼은 탭을 복제해 그룹을 추가하고, 각 그룹 내용을 레이아웃 트리 기준으로 렌더링한다. pane 클릭으로 활성 그룹을 바꾸며, 활성 pane을 닫을 수 있다. 그룹별 탭 strip/header와 divider 드래그를 구현했고, pane 비율은 layout split node의 `dimensions` 배열로 저장/복원한다. Web WASM `cargo check`가 통과했다. 실제 브라우저에서 드래그 및 새로고침 후 비율이 복원되는지 확인은 남아 있다.

각 pane의 tab strip에서 그룹 ID를 포함한 drag payload로 탭 순서를 조정하도록 했다. 비활성 pane에서 시작한 reorder도 해당 그룹을 활성화한 뒤 처리한다. Web의 파일 찾기·새 노트·dock 토글 키는 공통 command ID로 dispatch한다. Google 자동 세션 복원이 실패하면 GIS 오류를 숨기지 않고 재연결 안내와 함께 표시한다. 빈 탭 액션의 높이·타이포그래피·hover·키 힌트는 두 호스트 모두 같은 UI 컴포넌트를 쓴다. Dock의 표시/탭 선택 전이를 `WorkspaceDockState`로 공유하고, width drag의 최소 폭과 콘텐츠 여유 계산은 `app::ui::workspace_dock_width_at`으로 공유한다. Web 파일 트리에서 비-Markdown Drive 항목은 Google Drive 미리보기를 새 브라우저 탭으로 연다. 복원된 Graph/Search/Terminal 탭은 더 이상 editor를 잘못 표시하지 않으며, Search pane과 dock Search는 검색 인덱스·결과를 연결한다. Web Graph는 실제 공유 node/edge와 force simulation을 canvas에 렌더링하고 panel, pan/zoom 및 노트 열기까지 연결한다. 각 Web pane 헤더에 Desktop과 같은 force graph/radial-tree 선택 버튼을 제공하고, radial-tree 데이터 생성은 `crates/app::explorer`에서 공유한다. Graph 직전의 문서 모드는 같은 탭 안에서 기억한다. 남은 Graph 차이는 Desktop `graph.json` 설정 interop과 실제 브라우저 검증이다. Terminal은 native PTY host가 없어 Web에서 미지원 상태를 표시한다. Desktop/Web 런타임 비교는 남아 있다.

공유 `Layout::insert_beside`/`remove`가 Web workspace의 split `dimensions` 배열도 갱신하도록 했다. 같은 방향 pane을 추가하면 기존 pane 비율을 둘로 나누고, pane 제거 시 남은 비율 관계를 보존한다. Desktop은 기존 group dimension 처리 흐름을 유지한다. Desktop native와 Web WASM `cargo check`, `git diff --check`가 통과했다. 같은 비율로 pane을 나누고 닫는 런타임 확인은 남아 있다.

탭 활성화와 pin 토글의 상태 변경을 `crates/app::workspace::TabGroup` / `WorkspaceModel` API로 모았다. Desktop의 활성 탭 스냅샷과 Web 모델이 범위 검증을 포함한 동일한 탭 인덱스 전이를 쓴다. `cargo check -p desktop --jobs 1` 및 Web WASM `cargo check`가 통과했다.

Web은 공통 command ID `workspace:new-tab`, `workspace:close`, `workspace:next-tab`, `workspace:previous-tab`, `global-search:open`, `app:go-back`, `app:go-forward`, `markdown:toggle-preview`, `graph:open`, `editor:save-file`, `editor:open-search`의 키 바인딩과 dispatch를 제공한다. 현재 파일 검색은 공용 Editor의 find panel을 연다. 저장 키는 공통 workspace에서 `SaveRequested`를 내고 Drive 저장은 host가 처리한다. 닫기는 pinned 탭에서 Desktop과 같은 unpin 동작을 하고, 탭 순환/history 이동과 Graph 전환은 현재 pane에서 처리한다. Preview/Source 전환 시 source caret offset을 복원하며 Terminal pane은 문서 모드로 바꾸지 않는다. Web은 `.tft/hotkeys.json`의 사용자 재정의를 읽어 command palette와 키 dispatch에 반영하며, 바뀐 기본 단축키는 실행되지 않도록 필터링한다. Web WASM compile check와 `git diff --check`가 통과했다. 나머지 Web 단축키 parity와 브라우저 키 이벤트 확인은 남아 있다.

Web Search도 Desktop처럼 검색 결과 선택 없이 시작하고, 위/아래 이동은 파일과 일치 문장 행을 따라 목록 경계에서 멈춘다. Enter는 파일이나 선택 문장 위치를 열고, Escape는 먼저 선택을 해제한 뒤 Search dock을 Files로 돌린다. Left/Right는 선택한 파일을 접고 펼치며, 일치 문장 행에서 접으면 포커스가 파일 행으로 이동한다. 선택 행은 검색 결과 viewport 안으로 스크롤한다. WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다. 브라우저 실제 키 이벤트 동작 확인은 남아 있다.

Web의 `Cmd/Ctrl+1…8`은 현재 pane의 해당 탭으로 이동하고 `Cmd/Ctrl+9`는 마지막 탭을 선택하도록 공통 command ID 기반 바인딩을 추가했다. 인덱스 범위를 벗어나는 단축키 입력은 무시한다. 사용자 지정 hotkey 설정도 `.tft/hotkeys.json`에서 로드해 키 실행과 팔레트에 반영한다. WASM `cargo check`와 `git diff --check`가 통과했다. 브라우저 키 이벤트 확인은 남아 있다.

같은 pane 안의 탭 reorder에서 Desktop/Web이 `move_tab_to_boundary`를 사용한다. 픽셀 기반 drop 위치 산출은 각 host에 남는다.

Desktop의 그룹 간 탭 이동도 source 제거 시 `remove_tab_at`, target 삽입 시 `insert_tab_at_boundary`를 사용해 공통 탭 인덱스 규칙을 따른다. 빈 그룹 유지/닫기와 에디터 view 동기화는 Desktop host 효과로 남는다. Desktop/Web compile check 및 `git diff --check`가 통과했다.

Drive API 요청은 token 만료 시 GIS의 무화면 `prompt: "none"` 토큰 재요청을 한 번 시도하고, 만료 임박 시 먼저 갱신하도록 공통 요청 어댑터를 사용한다. OAuth 요청 callback이 오지 않는 경우에도 무기한 대기하지 않고 15초 후 실패 상태를 반환한다. 토큰 복원은 local/session storage를 각각 검사하므로 만료된 local copy나 저장소 접근 실패가 유효한 session copy를 지우지 않는다. Client ID/API key와 마지막 Vault를 저장하는 경로도 브라우저 저장소 예외를 삼키며, 설정 화면은 Client ID/API key 저장 실패를 사용자에게 알린다. Drive workspace 읽기가 실패하고 로컬 임시본도 없으면 저장본을 못 찾은 채 새 workspace를 여는 대신 오류를 보여준다. 로컬 임시본이 있으면 해당 상태로 열고 Drive 읽기가 실패했을 때도 재동기화를 시도하며, Drive 쓰기까지 실패하면 임시본을 보존한다. `drive.js` Node 구문 검사와 Web WASM `cargo check`, `git diff --check`를 통과했다. 실제 Google 계정에서 새로고침 및 401 갱신 동작은 아직 확인하지 않았다.

Web의 복원 Search view는 이제 공통 `workspace_search_view` shell에서 검색 입력과 결과/오류 상태를 렌더링한다. dock Search도 같은 shell과 결과 위젯을 사용하며, split pane 검색 결과는 그 pane의 그룹에 문서를 연다.

Desktop의 Prompt 컴포넌트를 `crates/app::prompt`로 이동해 양쪽 host가 같은 입력·버튼·오류 UI를 쓴다. Web Drive의 새 Markdown note와 새 폴더 작업은 이름 입력을 그 공통 모달에서 받고, Vault 검증 오류는 입력 중 재시도할 수 있게 표시한다. 브라우저 기본 `window.prompt` 사용은 제거했다. Desktop/Web WASM compile check와 `git diff --check`가 통과했다.

Desktop/Web의 문서 breadcrumb trail 조립을 `workspace_breadcrumb_trail`로 공통화했다. 두 호스트가 같은 버튼/구분선 순서를 쓰며 폴더 이동 동작은 각 host callback으로 유지한다. Desktop 및 Web WASM `cargo check`와 `git diff --check`가 통과했다.

Web 비-Markdown 문서도 이제 workspace 탭에 열리고 저장/복원된다. Image는 Drive 인증 blob URL을 탭 안에서 렌더링한다. PDF는 Web에서 Drive binary bytes를 받아 공용 `PdfView`와 `Documents` page cache에 제공한다. Canvas는 Drive JSON을 공용 `Scene`에 읽어 카드와 edge를 배치하고 휠/버튼 zoom in/out·fit, 배경 드래그 pan, edge 선택·방향 순환·label 표시·생성·편집·삭제, 카드 드래그 이동, Shift-click 다중 선택과 선택 묶음 이동, 선택 항목 그룹 생성, 카드 8방향 resize, text card 추가, 공유 `Editor` 텍스트 카드 더블클릭 편집, 선택 항목의 기본색/6개 preset 변경, 선택 항목 삭제, undo/redo를 revision 조건부로 저장한다. Label 입력은 공통 Prompt, 변경은 Scene transaction을 사용한다. Canvas별 확대율과 위치는 Web workspace 상태에 파일 경로 기준으로 기록하고 복원한다. 편집 완료나 문서 전환 때 편집 transaction을 닫고 저장한다. unknown JSON fields와 key order는 공용 Canvas serializer가 보존한다. Mermaid는 Web과 Desktop이 공용 Editor 및 `tft_mermaid` renderer를 사용한다. Audio는 브라우저 `Audio` player와 재생/일시정지·seek·mute·시간 표시를 제공한다. Video는 탭에서 인증 blob URL을 불러오고 재생 액션으로 브라우저 native `<video controls>` overlay를 연다. Web WASM compile check와 `git diff --check`가 통과했다. 실 Drive 편집/미디어 동작을 브라우저에서 확인하는 단계는 남아 있다.

Graph의 force-directed `Layout`/`Forces` 엔진과 그래프 data builder를 `crates/app`에 공유했다. Web Graph는 동일한 vault metadata 기반 node/edge를 GPUI canvas에 그리며, 백그라운드 force simulation, 자동 fit, pan/zoom, 노트 열기와 zoom threshold label을 제공한다. 필터 panel에는 text search, Tags, Attachments, Existing files only, Orphans, Arrows 토글, Display/Forces sliders 및 검색 기반 color groups를 연결했다. GraphConfig 전체/merge JSON serializer를 `crates/formats`에서 공유해 Web도 `.tft/graph.json`을 읽고 저장하며, 기존 Web workspace `graphConfig`는 마이그레이션 fallback으로 읽는다. 설정 변경은 local workspace 임시본과 Drive의 graph.json에 함께 저장하고, 동기화가 끝난 뒤 `tftWeb`에서 중복 GraphConfig를 제거한다. 그래프 설정 JSON의 미지 키, BOM, CRLF/LF를 보존한다. Node hover는 연결 edge와 노드 label을 강조한다. Local graph는 활성 노트를 중심으로 따라가며 Depth, incoming/outgoing/neighbor 링크 설정을 사용한다. 문서 편집 후 graph data도 debounce 재구축한다. Google/Drive 실 브라우저 새로고침 검증은 남아 있다. OAuth access token은 만료 정보와 OAuth Client ID를 함께 브라우저 저장소에 기록하고 새로고침·새 탭에서 복구한다. 만료 시 Google 세션 기반 무화면 GIS 재개를 시도하며, Google이 이를 거부하면 사용자 로그인이 필요하다. Workspace JSON은 브라우저 로컬 임시본에 즉시 기록하고 Drive 저장 성공 뒤 임시본을 지운다. Drive에 저장된 상태는 `.tft/workspace.json`에서 복원하며, 전송이 완료되지 않은 상태는 임시본에서 복원해 Drive에 다시 반영한다.

Desktop과 Web의 split divider 드래그가 동일한 `split_dimensions_at` 전이를 사용한다. 각 host는 포인터 좌표를 정규화하고 결과 비율을 자기 workspace 저장 형식에 반영한다.

파일 트리 행의 확장자 태그 표시를 `file_tree_row_tag`로 추출해 Desktop과 Web이 같은 타입 크기·색·간격을 사용한다. Desktop/Web WASM `cargo check`와 `git diff --check`가 통과했다.

Web Files 정렬 순서는 `tftWeb` workspace 상태로 저장하고 복원한다. Desktop과 Web 모두 네 개의 Files 툴바 동작과 관련 sort/expand 설정 UI를 두지 않는다. 기존 `workspace.json`에 남아 있는 `tft.fileTreeSort`와 `tft.fileTreeExpanded` 값은 알 수 없는 키로 보존하며 읽거나 적용하지 않는다.

Desktop과 Web 모두 Files 툴바의 네 버튼과 sort/auto-reveal/expand-all/toolbar-new-note 동작, sort 및 expanded-folder 저장·복원을 두지 않는다. 트리 행 펼치기와 컨텍스트 메뉴, 명령 팔레트의 새 노트 기능은 유지한다. 기존 workspace JSON 키는 읽기/쓰기에서 건드리지 않는다. Web 좌측 dock의 Files/Search 탭도 제거해 Desktop과 같은 Files 단일 dock으로 맞추고, Search는 메인 영역 탭으로 연다.

Web에서 `Mod+F`를 `editor:open-search` command로 dispatch해 Desktop과 같은 Editor find panel을 연다. `Mod+H`는 브라우저 History 단축키와 충돌하므로 Web에는 아직 등록하지 않았다. Web WASM compile check와 `git diff --check`가 통과했다.

Web command palette는 편집 가능한 Markdown에서 `editor:open-search-replace`도 노출하고, 공용 Editor의 같은 Find/Replace panel을 실행한다. `Mod+H` 직접 키 바인딩은 브라우저 단축키와 충돌해 추가하지 않았다.

Web 빈 탭에 Desktop과 같은 New canvas action을 추가했다. Drive에 빈 `.canvas` 파일을 만들고 공용 `VaultData`에 Canvas 타입으로 삽입한 뒤 Canvas 탭을 열며, 기존 vault 경로 충돌 시 Untitled 이름에 번호를 붙인다. Web WASM compile check가 통과했다.

명령 팔레트 컴포넌트를 `crates/app::command_palette`로 옮겼다. Desktop과 Web이 같은 fuzzy command ranking, 최근 명령, 키 힌트, 선택 UI를 사용한다. Web은 구현된 command 중 현재 pane/document 상태에서 실행 가능한 것만 목록에 제공하고 `Cmd/Ctrl+Shift+P`로 열며, 선택한 command를 Web host dispatcher로 실행한다. 현재 편집 가능한 Markdown에서는 `Search & replace`도 팔레트에서 실행할 수 있다. Split vertical/horizontal과 close pane은 그룹 수에 따라 표시하고, Close others와 Toggle pin은 현재 탭 상태를 반영해 실행한다. pinned 탭의 Close 명령도 Desktop과 같이 unpin 처리한다. Web WASM compile check 및 `git diff --check`가 통과했다.

Hover row의 window-space anchor 측정을 shared `app::ui`로 옮겨 Desktop page preview도 해당 helper를 사용한다. Web Files 행은 기본 page-preview modifier인 Cmd/Ctrl을 누르고 300 ms 머물면 같은 shared `ReadingView`/Markdown parser로 popover를 표시한다. Web의 Reading/Editor 내부 Markdown 링크도 `.tft/page-preview.json`의 `preview`/`editor` modifier 설정을 따르고, 300 ms 뒤 popover를 표시하며 heading/block subpath로 스크롤한다. 포인터가 행/링크와 popover를 벗어난 뒤 300 ms 후 닫는다. Preview 안 링크는 현재 preview 문서를 기준으로 클릭 이동/새 탭 열기를 하고, 부모 popup을 유지하며 최대 8단계까지 중첩한다. 각 레벨은 별도 shared `ReadingView`를 사용한다. Popover 본문 클릭은 같은 box의 shared `Editor`로 전환하고 클릭 위치에 caret을 둔다. 중첩 preview의 링크는 해당 문서를 기준으로 해석하며 Escape/바깥 클릭 시 dirty buffer를 Drive autosave에 넘긴다. 편집 내용은 같은 note가 열려 있는 pane/preview editor에도 selection 위치를 유지하며 동기화한다. Desktop native 및 Web WASM compile check와 `git diff --check`가 통과했다. 8단계 제한과 실제 브라우저 포인터/편집 비교는 남아 있다.

Page preview 진입 위치(`Reading`, `Editor`, `Files`, `TabHeader`, `Graph`)와 `page-preview.json`의 modifier 기본값 해석을 `crates/app::page_preview`로 옮겼다. Desktop과 Web이 같은 source enum 및 `requires_mod` 규칙을 사용하며, Web hover callback은 임의 boolean 대신 source를 넘긴다. Web Graph의 Markdown 노드 hover도 같은 preview flow와 Graph modifier 설정을 사용한다. Web WASM와 Desktop native `cargo check`, `git diff --check`가 통과했다. 실제 브라우저 포인터 동작 비교는 남아 있다.

Graph 엔진 선택 버튼의 아이콘과 선택 강조 표시를 `app::ui::workspace_graph_engine_button`으로 공유해 Desktop, Web 단일 pane, Web split pane이 같은 UI를 쓴다. Web 단일 pane 문서 헤더에도 force Graph/radial-tree 진입점을 제공하며 Graph 상태에서 선택된 엔진을 표시하고 현재 그룹만 전환한다. Desktop native 및 Web WASM `cargo check --features drive-smoke-test`, `git diff --check`가 통과했다. 실제 브라우저의 버튼 조작 및 화면 비교는 남아 있다.

Web 파일 트리의 우클릭 메뉴에 Desktop과 같은 “Search in folder”를 추가했다. 파일은 상위 폴더, 폴더는 해당 폴더를 대상으로 `path:"folder/"` query를 만들고, 현재 pane의 Search 탭을 열어 query를 넣고 포커스한다. 기존 Search 탭이 있으면 재사용한다. Web WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다. 실제 검색 결과 및 키보드 포커스는 브라우저에서 확인해야 한다.

Web의 활성 파일 탭 우클릭 메뉴에서 빠졌던 `Move file to...`와 `Delete file`을 기존 Drive 이동 prompt 및 삭제 확인 흐름에 연결했다. `Copy relative path`를 Desktop의 `Copy path`와 같은 라벨로 맞췄다. Web WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다. 실제 Drive 이동·휴지통 이동은 브라우저에서 확인해야 한다.

Web 파일과 폴더 우클릭 메뉴에 “New canvas”를 추가해 해당 Drive 폴더를 부모로 전달한다. 기존 Drive Canvas 생성 host flow가 이름 입력, 충돌 방지, 파일 생성과 Canvas 탭 열기를 처리한다. Web WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다. 실제 Drive에서 생성 결과를 확인하는 단계는 남아 있다.

Web 파일 우클릭 메뉴의 “Open to the right”는 현재 group을 세로 split한 뒤 새 pane에서 해당 파일을 연다. split 상한에 도달하면 메뉴 항목을 감춘다. 기존 split/tab 전이와 파일 로딩을 재사용하며 WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다. 실제 pane 전환과 파일 열기는 브라우저에서 확인해야 한다.

Web 파일 우클릭 메뉴에 “Open in Google Drive”를 추가해 OS 기본 앱 대신 브라우저의 Drive 파일 보기로 연다. 우클릭 메뉴가 늘어난 만큼 창 높이에 맞춰 최대 높이를 제한하고 내부 스크롤을 제공한다. Web WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다.

Web의 `global-search:open` command palette action과 단축키를 Desktop `open_search_action`과 맞춰, 기존 Search query를 보존하며 중앙 Search tab을 열고 query input에 포커스한다. Dock의 Search tab 선택은 그대로 둔다. Web WASM `cargo check --features drive-smoke-test`와 `git diff --check`가 통과했다.

Wikilink `#Heading`/`#^block` subpath에서 heading/block의 metadata offset을 찾는 순수 처리를 `crates/app::subpath_offset`으로 올렸다. Desktop과 Web이 같은 Markdown parser와 대소문자 무시 heading 비교를 사용하고, 두 host의 Editor/ReadingView가 해당 위치로 이동한다. Web WASM 및 Desktop `cargo check`, `git diff --check`가 통과했다.

`WorkspaceModel::split_with_active_tab`이 split 생성·활성화와 현재 탭 복제를 한 번에 처리하도록 추가했다. Web host는 editor buffer를 먼저 모델에 반영하고 새 그룹의 view만 생성한다. `WorkspaceModel::activate_group`도 추가해 Web 링크/검색/탭 이동과 restore에서 layout 밖 그룹을 활성화하지 않게 했다. Web WASM 및 Desktop `cargo check`, `git diff --check`가 통과했다.

Web이 각 문서를 열고 복원할 때 저장된 `LivePreview`/`Source`를 적용하도록 `Editor::set_workspace_document`를 추가했다. Web의 Reading↔LivePreview 전환은 Desktop과 같이 편집 buffer와 caret/history offset을 유지하고, editing/reading 간 fold 상태를 옮기며, 편집 화면에서 나갈 때 Drive 저장을 요청한다. 기본 편집 모드도 Desktop 설정과 같이 Live Preview로 맞췄다. Web WASM 및 Desktop `cargo check`, `git diff --check`가 통과했다.

Search 6종 결과 정렬에서 Web이 이름 동률을 경로 오름차순으로 처리하고 시간 동률에 이름 정렬을 추가해 Desktop과 달라지는 것을 확인했다. `crates/app::search_results::compare_search_results`를 Desktop/Web이 함께 사용하도록 바꿔 자연 이름 정렬, descending path tie-break, 시간별 stable ordering을 일치시켰다. Web WASM 및 Desktop `cargo check`, `git diff --check`가 통과했다.

Web은 Drive의 `.tft/app.json`과 `.tft/switcher.json` 설정을 로드한다. `userIgnoreFilters`를 Search와 Graph에서 제외하고, 빠른 파일 전환기에서는 제외 파일을 일반 결과 뒤에 낮은 강조로 표시하며 최근 목록에서 뺀다. `rank_candidates`로 이름/경로/alias 점수 계산, 최근 파일 목록, 제외 순위와 tie-break까지 공유해 Web과 Desktop의 빠른 파일 전환기 ranking을 통일했다. 파일 종류 필터와 첨부파일 표시 옵션도 Desktop 규칙과 같이 적용한다. Web WASM과 Desktop `cargo check`, `cargo fmt --check`, `git diff --check`가 통과했다.

빠른 파일 전환기의 매칭·ranking과 파일 종류 필터를 `crates/app::file_switcher`로 공유하고, Desktop/Web의 다음·이전 탭 이동 index 계산도 `cycle_tab_index`로 통합했다. Web WASM과 Desktop `cargo check`, `cargo fmt --check`, `git diff --check`가 통과했다.

Indexed `VaultData` 기반 `FileTree`가 unsupported 확장자 파일을 숨기지 않던 Desktop/Web 차이를 고쳤다. `from_vault_with_options`로 같은 표시 옵션을 전달할 수 있고 기본 경로는 Desktop의 현재 기본값(false)을 사용한다. Web WASM과 Desktop `cargo check`, `cargo fmt --check`, `git diff --check`가 통과했다.

Internal wikilink body parsing과 vault-relative destination resolution을 `crates/app::resolve_internal_link`으로 공유했다. Desktop과 Web은 host별 탭/파일 동작을 유지하면서 동일한 parsed path/subpath와 resolver 결과를 사용한다.

Web에서 대상이 없는 Markdown/Canvas wikilink를 클릭하면 해당 파일 종류의 Drive 생성 Prompt를 링크 이름으로 미리 채운다. 새 탭 링크는 먼저 새 탭을 만들고, 기본/current/configured-folder 위치 규칙을 `.tft/app.json`에서 읽어 적용한다. 링크 경로에 존재하는 하위 폴더를 부모로 쓴다. 존재하지 않는 중간 폴더 자동 생성은 아직 지원하지 않는다.

Web 두 종류의 tab strip에서 Markdown 파일 탭의 Cmd/Ctrl hover를 Files 행의 공용 preview 경로로 연결했다. 300 ms 뒤 같은 ReadingView로 미리 보고 pointer가 탭과 popover를 벗어나면 닫는다. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, `git diff --check`가 통과했다. Drive의 `.tft/page-preview.json`을 읽어 `tab-header` modifier 설정을 적용한다. 브라우저에서 실제 hover/anchor 위치와 설정별 동작을 확인해야 한다.

Web command palette에 `graph:open-local`을 추가해 Desktop과 같은 command ID로 실행한다. 활성 파일이 있으면 현재 파일을 local graph 중심으로 지정하고 Graph pane을 연다. 이미 Graph가 활성인 경우에는 pane의 local mode를 전환하고 상태를 저장한다. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, `git diff --check`가 통과했다.

Web 편집기가 활성 Markdown 문서의 Live Preview/Source 상태일 때 공용 `Editor` API로 `editor:toggle-bold`, `editor:toggle-italics`, `editor:toggle-comments`, `editor:toggle-checklist-status`, `editor:insert-link`를 실행한다. 해당 command ID에 Desktop 기본 Cmd/Ctrl 단축키를 연결하고 command palette에도 조건부 노출했다. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, `git diff --check`가 통과했다. 브라우저의 키 이벤트와 편집기 선택 상태별 결과는 런타임 확인이 남아 있다.

Web은 `Mod+D`를 Desktop 기본 command `editor:delete-paragraph`에 연결했다. 활성 Markdown 편집 시 공용 Editor의 문단 삭제 동작을 실행하고 command palette에도 편집 가능한 경우 노출한다. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, `git diff --check`가 통과했다.

Web Markdown editor now dispatches Desktop's `editor:follow-link`, `editor:open-link-in-new-leaf`, and `editor:open-link-in-new-split` IDs from Alt+Enter, Mod+Enter, and Mod+Alt+Enter. Internal paths continue through shared `resolve_internal_link`; new leaf reuses the existing-tab/new-tab link flow, and new split uses `WorkspaceModel` split before resolving. External URLs open through the browser host and tags route to Search. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass; actual browser key and caret behavior remains to verify.

Web command palette now supports `file-explorer:open` and `file-explorer:reveal-active-file`, selecting Files, revealing the active Vault path and focusing the tree. `file-explorer:new-file` follows Desktop behavior by opening a new workspace tab when a file is active before showing the Drive creation prompt. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass.

Web command palette now exposes the Markdown editor operations already implemented by the shared `Editor`: strikethrough/highlight/code/math, bullet/numbered/blockquote prefixes, formatting clear, table and block insertion, headings, list indent, line swaps, and folding. They dispatch through the common Editor APIs and only appear for an editable Markdown tab. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass. Runtime selection/caret parity remains unverified.

Web command palette now includes `editor:toggle-source`, switching the active Markdown editor between Source and Live Preview while updating the current history entry and preserving its caret offset. Authentication resume persists the short-lived OAuth access token with expiry and OAuth client ID in browser storage, then attempts Google's silent token flow after expiry; workspace state is written immediately to local storage and synced into `.tft/workspace.json` in Drive. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass. Browser behavior and a real Google account session still need runtime confirmation.

Web command palette now exposes the shared Markdown property commands for editable notes: add an unnamed metadata row, add an alias, clear metadata properties, and fold/unfold properties. Alias and property removal update the shared Vault buffer and request Drive save; property folding also works in Reading mode. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass. Drive persistence still needs browser runtime confirmation.

Web's Markdown command palette now offers the Desktop `editor:insert-wikilink` and `editor:insert-embed` actions in addition to Markdown link insertion. All three use the shared Editor's `LinkSyntax` path and are limited to editable Markdown tabs. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass.

Web command palette now includes Desktop's table row/column operations (insert, move, copy, delete, and column alignment). It shows only operations that the shared Editor reports as applicable at the current caret/table selection, then dispatches through `Editor::table_command`. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass. Drive autosave and table-selection interaction need browser runtime verification.

Web document-header titles now use the shared inline field for click-to-caret editing and F2 selection. Enter validates a same-folder Markdown rename and requests the Drive host operation. The host uses the shared metadata cache to plan link rewrites, asks before updating linked files unless `alwaysUpdateLinks` is enabled, renames the Drive object, writes accepted rewrites, and refreshes Vault paths, tab/history paths, Files tree, search index, open editor/reading views, and graph. Web WASM `cargo check --features drive-smoke-test`, `cargo fmt --check`, and `git diff --check` pass. Runtime title input, link prompt, and Drive persistence still need browser verification.

Workspace 자동 저장은 localStorage 임시본을 즉시 저장하고 Drive에 450 ms debounce로 동기화한다. 브라우저 저장소 quota/security 예외를 잡아 WASM 경계 밖으로 전파하지 않고, Drive 저장 성공/실패와 임시 저장소 미사용 상태를 앱 상태에 표시한다. 임시 저장소 읽기/삭제도 예외에 안전하게 처리한다. `node --check crates/drive-web/drive.js`, Web WASM `cargo check --features drive-smoke-test`, `git diff --check`가 통과했다. 새로고침 후 실제 Google 계정으로 복원하는 런타임 확인은 여전히 남아 있다.

Web의 `Save to Drive` 문서 도구는 현재 탭이 Markdown일 때만 표시한다. 저장 host가 Markdown 이외 문서를 거부하는 상태에서 Canvas·PDF·미디어 탭에 노출되던 실패 동작을 제거했다. Drive 저장 버튼이 없는 문서 종류는 기존 autosave/Canvas별 저장 흐름을 사용한다. Web WASM compile check와 `git diff --check`가 통과했다.

Web command palette와 hotkey dispatcher에 Desktop의 `file-explorer:new-folder` command를 연결해 Drive 폴더 생성 prompt를 연다. `editor:add-cursor-above`와 `editor:add-cursor-below`는 공용 Editor의 multiple-cursor API로 실행하고 편집 가능한 Markdown일 때만 command palette에 노출한다. Web WASM compile check와 `git diff --check`가 통과했다.

Web command palette의 `file-explorer:duplicate-file`과 `app:delete-file`을 기존 Drive duplicate/휴지통 흐름에 연결했다. Duplicate는 이름 충돌 검사와 Drive copy 후 인덱스 반영을, Delete는 확인·미저장 보호 후 휴지통 이동과 workspace 정리를 사용한다. 실제 Drive 조작을 제외한 Web WASM compile check와 `git diff --check`가 통과했다.

Web `editor:rename-heading`을 command palette와 dispatch에 연결했다. 공용 Editor가 caret 아래 ATX heading을 확인하고, 공용 Prompt가 이름을 받아 `MetadataCache::plan_heading_rename`으로 heading과 해당 Vault 링크의 byte-range 수정을 계획한다. 적용은 Editor의 단일 undo 편집으로 처리하며 연결 문서의 열린 pane, 검색 메타데이터, Graph와 Drive autosave도 갱신한다. Web WASM compile check와 `git diff --check`가 통과했다. 런타임 커서/링크·Drive 결과 비교는 남아 있다.

Web command palette와 hotkey dispatch에 `editor:focus-left/right/top/bottom`을 추가했다. 공유 `Layout::neighbors`로 현재 split tree의 방향상 이웃 pane을 찾고, Desktop처럼 최근 활성 pane을 우선한다. 실제 neighbor가 없는 command는 palette에 표시하지 않으며, 선택된 pane의 Reading/Editor view에 포커스를 복원한다. Web WASM compile check와 `git diff --check`가 통과했다.

닫힌 탭 복원 이력의 20개 제한과 LIFO 순서를 `crates/app::ClosedTabHistory`로 공유하고 Web의 `workspace:undo-close-pane` command palette 액션을 연결했다. 탭을 닫거나 여러 탭을 닫을 때 현재 뷰의 history offset을 기록하고, 복원 시 원래 삽입 index와 문서 모드/offset을 되살린다. 탭 활성화·재정렬, pane 전환, split, pane 간 탭 이동, 링크 이동, back/forward도 같은 `stash_group_view_state`를 통해 offset을 저장하며, 탭 재활성화와 Reading history 이동 때 해당 위치로 이동한다. 브라우저의 탭 복원과 충돌하는 Mod+Shift+T는 Web에서 바인딩하지 않고 팔레트 항목의 단축키 힌트도 감춘다. Desktop/Web WASM `cargo check`, Desktop `cargo check -p desktop`, rustfmt와 `git diff --check`가 통과했다. 실제 브라우저 조작 검증은 남아 있다.
