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
- [x] 빈 `Tab` 모델을 빈 화면으로 판별하고 Web의 새 노트/파일 찾기 액션을 연결한다.
- [ ] 공통 위젯의 크기, 색상, hover, 키 힌트를 Desktop의 기준과 맞춘다.

### 1. Workspace shell과 탐색

- [x] 좌측 dock의 공통 base geometry와 배경을 `app::ui`로 옮기고 양쪽 호스트에서 사용한다.
- [x] 좌측 dock 상단 header row의 공통 높이·배경·구분선을 양쪽 호스트에서 공유한다.
- [x] Web dock 접기와 탭 strip 복원 진입점을 연결한다.
- [ ] 좌측 dock 크기·접기·탭 선택·하단 Vault profile 레이아웃을 공통 shell로 옮긴다.
- [ ] 파일 트리, 검색, 정렬, 새 파일 및 파일 행의 공유 UI와 상태 전이를 연결한다.
- [x] 파일 행의 chevron, 들여쓰기 가이드와 긴 이름 말줄임을 공유한다.
- [x] 파일 트리에서 Ctrl/Command 클릭으로 새 탭을 여는 동작을 Desktop과 맞춘다.
- [x] Web 파일 트리에 포커스 가능한 키보드 탐색(위/아래, 좌/우, Enter, Space, Escape, Tab)을 연결한다.
- [x] Up/Down 시작 행과 Left/Right 폴더 전이를 `FileTree`에서 공유하고 Desktop/Web이 함께 사용한다.
- [x] Web 파일 트리에 지원되는 우클릭 메뉴 동작(새 노트, 새 탭 열기, 탐색기에서 표시, 경로 복사, 폴더 펼치기/접기)을 연결한다.
- [ ] Desktop의 native frame, OS 메뉴와 Web의 브라우저/Drive 제어는 host slot으로 남긴다.

### 2. 탭과 문서 헤더

- [x] 탭/헤더 제목의 공통 표시 규칙을 `crates/app`으로 옮기고 Desktop/Web에서 사용한다.
- [x] 탭 strip의 외부 프레임 높이, 배경, clipping과 하단 hairline을 공통화한다.
- [x] 새 탭 추가 전이를 `crates/app`에 두고 Desktop/Web의 `+` 버튼에서 사용한다.
- [x] 탭 내부 여백, 비활성 hover, hover 시에만 보이는 닫기 아이콘을 양쪽 앱에서 공유한다.
- [x] 탭 strip의 가로 스크롤 viewport를 양쪽 앱에서 공유한다.
- [x] 탭 strip의 뒤로/앞으로 컨트롤 셀과 탭 사이 구분선을 양쪽 앱에서 공유한다.
- [x] 폴더 breadcrumb 데이터와 표시 스타일을 공유하고 Web에서 클릭해 탐색기를 해당 폴더로 이동한다.
- [x] 모드 전환 버튼 표시 조건을 공유해 빈 탭과 지원하지 않는 view에 숨긴다.
- [x] Web에서 중간 클릭으로 탭을 닫을 수 있게 한다.
- [ ] 탭 strip 레이아웃, 활성/비활성 스타일, overflow, 추가·닫기 및 탭 전환을 공유한다.
- [ ] 뒤로/앞으로, breadcrumb/title, view actions의 구조와 공통 동작을 공유한다.
- [ ] 드래그, 우클릭 메뉴, 창별 탭 persistence 중 호스트 의존 부분을 분리한다.

### 3. 문서 화면

- [x] 같은 Editor와 ReadingView 구현을 두 호스트에서 사용한다.
- [ ] Preview와 문서별 도구 동작을 두 호스트에서 맞춘다.
- [ ] 링크 이동, 선택/커서, 편집/저장 전이가 공유 앱 API를 통하도록 정리한다.
- [ ] PDF, Canvas, Graph, Mermaid와 미디어는 지원 범위를 확인하고 단계적으로 포팅한다.

### 4. 분할, 상태 및 보조 UI

- [ ] `WorkspaceModel` 레이아웃을 기준으로 split/group 전환과 크기 조절을 공유한다.
- [ ] 상태 표시줄, 검색/명령 대화상자, 메뉴, prompt, hover preview를 공통 위젯으로 맞춘다.
- [x] Web의 파일 선택 진입점에 공통 팔레트 UI, 이름/경로/alias 검색, 최근 파일, Cmd/Ctrl+P, 위/아래·Enter·Escape를 연결하고 이름 매칭 점수를 Desktop과 공유한다. 현재 Drive Web이 여는 Markdown 파일만 결과로 제공한다.
- [ ] 단축키는 공통 command ID를 사용하고 host별 입력 바인딩만 어댑트한다.

### 5. 통합 및 parity 정리

- [ ] 동일한 Vault fixture와 동일한 workspace 상태에서 Desktop/Web 화면을 비교한다.
- [ ] 각 단계의 사용자 조작을 양쪽에서 확인하고 미지원 동작은 명시한다.
- [ ] Specification 및 관찰 동작과 어긋나는 부분은 probe/fixture 근거를 추가해 갱신한다.
- [ ] 빌드·검증 결과를 기록한다. 요청되지 않은 테스트는 실행하지 않는다.

## 완료 기준

- 기본 workspace의 정보 구조와 조작 결과가 Desktop/Web에서 일치한다.
- 공통 UI와 전이는 `crates/app` 한 곳에 있고 양쪽 앱이 이를 사용한다.
- 파일 저장, 브라우저/OS API, 인증 등 플랫폼 효과는 명확한 host 경계에 남는다.
- 포팅되지 않은 기능과 검증되지 않은 동작을 완료로 표시하지 않는다.

## 현재 작업

0단계에서는 Web의 빈 tab 판정 오류를 고쳐 초기 화면이 편집기 빈 화면 대신 액션 목록을 그리게 했다. 1단계에서는 좌측 dock의 기본 geometry와 header row를 `app::ui`로 추출하고 `WorkspaceModel.left_visible`으로 접기/복원을 관리한다. 파일 트리는 chevron·indent 가이드·긴 이름 말줄임을 공유하며, `FileTree`의 키보드 이동 전이를 Web과 Desktop이 함께 사용한다. Web 우클릭 메뉴에는 현재 지원되는 새 노트, 새 탭 열기, 탐색기에서 표시, 경로 복사, 폴더 접기/펼치기를 연결했다. 2단계에서는 탭 strip 외부 프레임, 탭 제목 규칙, 빈 탭 추가 전이, 탭 내부 스타일, 가로 스크롤 영역, 히스토리 컨트롤 셀, breadcrumb 표시 및 폴더 탐색 동작을 공유한다. 3단계에서는 Desktop/Web이 동일한 Editor와 ReadingView 구현을 사용함을 확인했다. Web의 빈 탭 “Go to file”과 Cmd/Ctrl+P는 공유 팔레트 위젯을 사용하는 파일 선택창을 연다. 이름 매칭의 exact/prefix/word-start/substring/fuzzy 점수화는 `crates/app::file_switcher`로 옮겨 Desktop과 Web에서 공유한다. Web은 Markdown 파일의 이름/경로/alias 검색과 현재 세션의 최근 파일 순위를 제공한다. Desktop switcher의 제외 파일 순위는 아직 이식되지 않았다. Web wasm `cargo check`는 `RUSTC_BOOTSTRAP=1`로 통과했다. 안정 채널 기본 검증은 `wasm_thread`의 `#![feature(stdarch_wasm_atomic_wait)]`가 nightly를 요구해 막혔다. 런타임 화면 비교는 아직 남아 있다.
