# Browser Pane

Desktop 전용 Browser를 **Terminal과 같은 독립 pane/tab**으로 포팅한다. Vault 문서가 아닌 leaf 하나이며,
주소·뒤로/앞으로·새로고침·제목·쿠키를 스스로 가진다.

- 선행 검증: [lkh0310kr/workspace-rust](https://github.com/lkh0310kr/workspace-rust) (`e53206e`) — gpui에
  브라우저를 임베딩하는 두 방식을 실제로 측정한 결과. 이 문서는 그 결과를 TFT 구조에 맞게 옮기는 계획이다.
- 핵심 결론: **CEF(Chromium) off-screen rendering(OSR) + CEF 156.0.31**을 쓴다. wry(WKWebView) child
  view는 gpui 위에 네이티브 뷰가 얹혀 z-order/클립/오버레이가 깨지고, 팝업·3rd-party 쿠키·Chrome UA도
  통과하지 못한다(`COMP`, `AUTH-004~007`).
- Browser는 **Desktop only**다. `apps/web`은 지금 Terminal과 같이 "Browser is available in Desktop"
  안내 leaf를 그린다.

## 1. 목표와 비목표

목표

- Terminal처럼 `browser:open`으로 새 tab/leaf를 열고, split/popout/닫기/포커스가 기존 workspace 규칙을 따른다.
- 웹 콘텐츠는 gpui 씬 안의 이미지로 그려져 command palette, 메뉴, 모달, 툴팁이 웹 위에 정상적으로 뜬다.
- 주소 입력, back/forward, reload, 페이지 제목, 로딩 상태, 찾기, zoom, 새 창(`window.open`), `<select>` 팝업,
  다운로드, IME/한글 입력, 컨텍스트 메뉴가 동작한다.
- 로그인 세션(쿠키)과 localStorage/IndexedDB가 앱 재시작 후 유지된다.
- `workspace.json`에 Browser leaf의 URL/제목이 저장되어 vault를 다시 열 때 복원된다.

비목표 (이번 단계)

- Web(WASM)에서의 브라우저. CEF는 네이티브 전용이다.
- 브라우저 내부의 다중 탭. workspace tab 하나가 브라우저 인스턴스 하나다.
- 북마크/방문 기록 UI, 확장 프로그램, passkey.
- Linux/Windows 1차 구현. macOS를 먼저 끝내고 플랫폼 경계를 유지한 뒤 확장한다.

## 2. 왜 CEF OSR인가

workspace-rust의 실측 요약:

| 항목 | wry (WKWebView child NSView) | CEF 156 OSR |
|---|---|---|
| gpui 오버레이가 웹 위에 그려짐 | ❌ 네이티브 뷰가 위에 얹힘 | ✅ gpui 이미지 |
| 오버레이 클릭이 gpui로 전달 | ❌ | ✅ |
| 리사이즈/클립/라운드/숨김 | ❌ | ✅ |
| OAuth 팝업 / `target=_blank` / 3rd-party 쿠키 | ❌ | ✅ |
| Chrome UA·`window.chrome` | ❌ | ✅ (`AUTH-007`) |
| Google 로그인 | — | CEF 151 크래시, **156.0.31에서 통과** (`AUTH-009`) |
| 입력/IME/스크롤/클립보드 | — | ✅ (`INPUT-001~027`) |
| CEF 자체 한계 | — | passkey 🚫, HTTP Basic ❌, key repeat ❌, 핀치 줌 TODO |

따라서 제품 경로는 **CEF 156.0.31 바이너리 + cef-rs 156.7.0 + OSR** 하나로 정한다. Cargo 의존성은
`cef = "=156.7.0"`으로 고정한다.
`151.x`는 Google 로그인에서 프로세스가 크래시하므로 절대 쓰지 않는다.

## 3. TFT 구조에 놓기

Terminal과 동일한 3층 구조를 쓴다.

```
crates/browser          엔진 중립: URL 정규화, pane 상태, toolbar/view, 입력 이벤트 타입, 테스트
crates/browser-cef      desktop 전용: CEF 초기화/OSR/이벤트/쿠키/팝업, cef 156.7.0
apps/desktop/src/workspace/browser.rs
                        BrowserPaneState, tab viewState(browser id/url/title), load/restore/close
apps/web                Browser leaf 안내 (Terminal과 동일)
```

- `crates/app`에는 pane의 **상태 전이**만 둔다: `ViewMode::Browser`, tab leaf 제목/아이콘 규칙,
  `browser:*` command 목록. CEF 타입은 절대 `crates/app`이나 wasm 대상에 들어오지 않는다.
- `apps/desktop`는 Terminal pane과 같은 방식으로 session map을 소유한다:

```rust
struct BrowserPaneState {
    views: HashMap<u64, BrowserView>,       // tab.viewState.browser
    next_id: u64,
    subscriptions: HashMap<u64, Vec<Subscription>>,
}
```

- tab 하나 = CEF `Browser` 하나. `viewState`는 `{"browser": 7, "url": "https://…", "title": "…"}`.
  Terminal의 `titleLocked`와 같은 사용자 이름 고정 규칙을 재사용한다.
- `browser:open` / `browser:new`는 `Tab::empty(ViewMode::Browser)`를 `place_new_tab`으로 넣고,
  `load_tab`이 view를 없으면 만든다. `browser:close`는 view를 drop해 CEF browser를 닫는다.
- `workspace.json` leaf type은 `"browser"`:

```json
{"id":"…","type":"leaf","state":{
  "type":"browser",
  "state":{"url":"https://example.com","title":"Example"},
  "icon":"lucide-globe","title":"Example"
}}
```

복원 시 `layout_read::tab_from_leaf`가 `ViewMode::Browser` + `view_state.url`로 tab을 만든다.
Terminal은 shell을 복원하지 않지만, Browser는 URL을 복원한다(사용자가 기대하는 동작).

## 4. CEF 런타임 통합

### 4.1 시작/종료

1. GPUI `Application::run` 전에 CEF를 초기화한다.
2. `cef::execute_process`로 서브프로세스(`--type=…`)를 먼저 처리한다. macOS는 별도 helper 앱이 필요하다.
3. `Settings { windowless_rendering_enabled: true, external_message_pump: true, … }`.
4. GPUI 타이머(약 4ms)에서 `do_message_loop_work()`를 호출한다. CEF 콜백은 메인 스레드에서만 돈다.
5. 종료 시 cookie manager `flush_store` + `cef_shutdown`. (spike `AUTH-013`: flush 없으면 쿠키 유실)
6. macOS: GPUI의 `GPUIApplication`에 `CefAppProtocol`/`CrAppProtocol` 메서드를 런타임 주입한다
   (spike `macapp.rs`). 이 패치는 macOS CEF에 필수다.

### 4.2 화면

- BrowserView는 `canvas`의 prepaint에서 CEF surface(BGRA)를 RGBA `RenderImage`로 올리고
  `window.paint_image(bounds, …)`로 그린다. 이전 generation 이미지는 `window.drop_image`.
- surface는 view별 1개 + popup(`window.open`) 1개 + widget(`<select>`) 1개. popup/widget은 gpui 레이어로
  같은 BrowserView 안에 배치한다.
- 배경이 없는 페이지는 OSR 기본이 투명이므로 흰색으로 명시한다(spike `COMP-006`).
- 크기/scale factor 변경 시 CEF `was_resized`; DIP와 physical pixel을 분리한다.
- 숨겨진 tab은 `was_hidden(true)`로 렌더를 멈추고, 보일 때 재개한다(spike `NAV-013`).

### 4.3 입력/포커스

- `BrowserView`가 `Focusable + EntityInputHandler`를 구현하고 마우스/휠/키/IME를 CEF로 보낸다.
- 좌표는 window → view → CEF DIP 변환, zoom factor를 곱한다(spike `INPUT-024`).
- 키는 spike `keys.rs`의 규칙을 그대로 쓴다:
  - macOS 기능 문자를 함께 보내야 편집 키가 한 번만 동작한다.
  - `RAWKEYDOWN` 단독은 두 번 동작, Space는 CHAR 필요.
  - Cmd+←/→는 OSR로 전달되지 않아 Home/End로 매핑.
  - key repeat(`e.repeat`)은 CEF 156에서도 실패(spike `INPUT-031`) — P1에서 우회/업스트림 확인.
- 앱 단축키와의 경계: Browser pane이 포커스여도 TFT command(`cmd-p`, `cmd-\`, pane split 등)는 gpui가
  먼저 처리한다(spike `INPUT-008` 통과). Browser 전용 단축키는 `Browser` key context에 둔다:
  `cmd-l` 주소창, `cmd-r` reload, `cmd-[`/`cmd-]` back/forward, `cmd-f` 찾기, `cmd-shift-i` DevTools.
- 포커스가 주소창(TextInput)과 웹 콘텐츠 사이를 오갈 때 CEF `set_focus`/`was_focused`를 맞춘다
  (spike `INPUT-036` TODO — TFT에서 P0로 올려 검증).

### 4.4 팝업/다이얼로그/메뉴/다운로드

- `window.open`은 `on_before_popup`에서 허용하고, popup browser를 OSR로 만들어 gpui 중앙 레이어에 그린다.
  opener 관계는 CEF가 유지한다(spike `COMP-009`, `NAV-009~011`).
- `<select>`는 CEF widget surface를 페이지 좌표에 얹는다(`COMP-007/008`).
- 컨텍스트 메뉴는 네이티브 메뉴를 끄고 gpui `menu`로 그린다. 링크/이미지/선택 텍스트/입력칸 항목은 P1.
- JS alert/confirm/prompt, beforeunload는 TFT `Prompt`로 처리한다(spike `UI-003/004`).
- 다운로드는 `~/Downloads`를 기본으로 하고, 저장 위치/진행률/취소 UI는 P1.
- 외부 프로토콜(`mailto:`, `tel:`)은 `cx.open_url`로 OS에 넘긴다.

## 5. 프로필/저장소

- CEF profile은 vault 밖에 둔다: `~/Library/Application Support/TFT/browser` (Linux는 XDG data).
- vault별 분리는 하지 않는다. 일반 브라우저처럼 하나의 프로필을 공유한다.
- 쿠키/캐시/localStorage/IndexedDB는 CEF profile이 관리하고, 주기적으로 cookie flush를 한다.
- `tft browser clear-data`(또는 설정 메뉴)로 프로필 초기화를 제공한다.
- Vault에는 URL/제목만 `workspace.json`에 저장한다. 페이지 내용은 저장하지 않는다.

## 6. 명령과 hotkey

`terminal:*`과 같은 모양으로 `crates/commands/data`에 등록한다.

| command | 동작 |
|---|---|
| `browser:open` | 현재 group에 Browser tab을 열고 주소창 포커스 |
| `browser:new` | 새 Browser tab (같은 group) |
| `browser:toggle` | Browser tab이 active면 직전 non-browser tab으로, 아니면 open |
| `browser:close` | active Browser tab 닫기 |
| `browser:back` / `browser:forward` / `browser:reload` | CEF navigation |
| `browser:focus-address` | 주소창 편집 |
| `browser:find` | 페이지 내 찾기 bar |
| `browser:devtools` | DevTools 패널 (P1) |
| `browser:open-external` | 시스템 기본 브라우저로 현재 URL 열기 |
| `browser:rename` | tab 이름 고정 (Terminal과 동일) |

기본 hotkey는 `terminal:open` 계열과 겹치지 않게 정한다. 사용자가 `hotkeys.json`으로 바꿀 수 있어야 한다.

## 7. 구현 단계

### P0 — pane이 실제로 뜨고 쓸 수 있음

1. `crates/browser`: URL 정규화(`example.com` → https, 검색어 → 검색 URL), Browser 상태, toolbar/view,
   단위 테스트.
2. `crates/browser-cef`: cef 156.7.0 로드/초기화/서브프로세스, OSR surface, navigation,
   mouse/key/IME/scroll, popup/select, cookie flush, macOS CEFAppProtocol 패치.
3. Desktop 통합: `ViewMode::Browser`, `BrowserPaneState`, `browser:*` command, leaf persistence/restore,
   split/popout/close, view header 아이콘(`globe`), `apps/web` 안내 leaf.
4. 패키징: `TFT.app`에 CEF framework/helper/resources를 넣고 nested code signing. CEF는 저장소에
   커밋하지 않고 `CEF_PATH`(cef-rs `export-cef-dir`)에서 가져온다.
5. 테스트 사이트로 다음을 자동 검증(창 없는 모드 우선):
   `COMP-001~009`, `NAV-001~014`, `INPUT-001~027` 중 P0, `STORE-001~008`, `UI-001~005`.
   기존 control socket에 `browser.navigate/state/screenshot`을 추가해 자동화한다.

Acceptance

- `browser:open` → 주소 입력 → 페이지 표시. command palette와 메뉴가 웹 위에 뜨고 클릭된다.
- split/popout/탭 전환/닫기에서 CEF surface와 포커스가 따라간다.
- 앱 재시작 후 쿠키 세션과 `workspace.json`의 URL/제목이 복원된다.
- 로컬 테스트 사이트에서 위 P0 케이스가 통과한다.

### P1 — 일상 품질

- 컨텍스트 메뉴 전체 항목, 찾기 UI, zoom(Cmd+/−/0, 핀치), 다운로드/저장 대화상자, 권한 프롬프트.
- DPI/다중 모니터, 창 비활성/활성 포커스, IME 후보 창 위치, key repeat 우회.
- 외부 로그인(GitHub/Apple/Microsoft/네이버/카카오)과 2단계 인증 수동 검증.
- DevTools 패널(두 번째 windowless browser) 또는 외부 DevTools.
- Linux CEF 빌드, Windows는 그다음.

### P2 — 제품화

- CEF sandbox 활성화(helper entitlements, 서명/노타리제이션), 프로필 clear-data UI.
- 브라우저 기록/북마크, 다운로드 관리, 인쇄/PDF.
- 앱 크기/첫 실행 시간 최적화(CEF lazy init, feature flag).

## 8. 패키징

- CEF 배포본은 크고(수백 MB) 저장소에 넣지 않는다. 빌드 머신에 `CEF_PATH`를 두고, 릴리즈 번들에만 포함한다.
- `TFT.app/Contents/Frameworks/Chromium Embedded Framework.framework`, `Contents/Frameworks/TFT Helper.app`,
  `Contents/Resources`(pak, icudtl, locales, v8 snapshot)를 넣는다.
- 서명 순서: helper → framework → app. helper에는 hardened runtime과 필요한 entitlements
  (`com.apple.security.cs.allow-jit`, `allow-unsigned-executable-memory`)를 준다.
- 기존 `tools/macos-install.sh`는 Cargo feature가 켜져 있으면 CEF 번들 단계를 추가한다.
  CEF가 없으면 Browser를 빼고 빌드하고 그 사실을 출력한다.
- macOS 배포는 Developer ID + notarization이 필요하다. 지금의 ad-hoc 설치 스크립트는 로컬 업데이트용이다.
- Linux는 CEF `.so`/`chrome-sandbox`, Windows는 `libcef.dll`/helper를 같은 원칙으로 둔다.

## 9. 보안/개인정보

- Browser는 임의의 웹 콘텐츠를 실행한다. CEF 샌드박스를 P1에서 켠다. 그 전까지는 내부/테스트 용도로만
  쓰고, 배포 기본값으로 Browser를 켜지 않는다.
- 다운로드/파일 선택/카메라/마이크/알림/위치는 기본 거부 + 명시적 허용.
- 프로필 쿠키는 vault와 분리하고, clear-data를 제공한다.
- Browser 페이지 내용은 vault나 검색 인덱스에 들어가지 않는다.
- CEF 라이선스(BSD)와 Chromium 고지문을 앱 bundle에 포함한다.

## 10. 리스크

| 리스크 | 대응 |
|---|---|
| CEF 바이너리 크기/첫 실행 시간 | feature flag, lazy init, 릴리즈에만 포함 |
| macOS 서명/노타리제이션 복잡도 | nested signing 순서 고정, helper entitlements, `codesign --verify --deep --strict` gate |
| CEF 156 고정 | 업그레이드는 Google 로그인/입력 회귀를 포함해 재검증 |
| passkey 미지원 | 비밀번호/Try another way 안내 |
| HTTP Basic 실패(`UI-014/015`), key repeat(`INPUT-031`) | P1에서 upstream/우회 조사 |
| GPUIApplication CEF 프로토콜 패치 | macOS 전용 초기화 코드로 격리하고 시작 시 검증 |
| 메시지 펌프 CPU | 4ms 타이머, 유휴 시 backoff |
| Web과의 기능 차이 | Terminal과 같은 "Desktop only" 안내 leaf, command는 Web에서 비활성 |

## 11. 참고 파일

- Terminal pane 선례: `apps/desktop/src/workspace/terminal.rs`, `crates/terminal/`
- tab/leaf 저장/복원: `apps/desktop/src/workspace/layout_read.rs`, `layout_write.rs`, `groups.rs`
- shared pane mode/command: `crates/app/src/workspace.rs`, `crates/commands/data/`
- CEF 검증 코드: `crates/spike-cef/src/cef_adapter.rs`, `cefhost.rs`, `application.rs`, `keys.rs`, `ui.rs`, `macapp.rs`
- 검증 결과 표: workspace-rust `docs/VERIFICATION.md` (COMP/AUTH/INPUT/NAV/STORE/UI)
