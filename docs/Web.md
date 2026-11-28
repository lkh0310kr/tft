---
tags:
  - tft/meta
---
# 기존 Leptos 웹 구현

이 문서는 `apps/web-leptos`에 보존된 Leptos 셸을 설명합니다. 현재 진행 중인 GPUI Web 구현과 단계별 계획은 [[Web-Rebuild]]에서 관리합니다.

웹 앱은 동일한 Rust Core를 사용하는 두 번째 셸이며, 기능을 축소한 제품이 아닙니다. 브라우저 뷰포트 안에서는 Desktop과 디자인 토큰, 레이아웃 치수, 동작이 일치해야 합니다. 뷰포트 밖의 브라우저 UI만 허용되는 차이입니다.

## 저장소

- Google Drive만 사용하며 OAuth scope는 `drive`입니다. Vault는 Picker로 고른 폴더이고, `drive.file`은
  Picker가 반환한 항목만 권한을 주므로 폴더 안의 기존 노트 쓰기/휴지통 이동에는 `drive`가 필요합니다.
- Picker에서 Vault 폴더를 선택합니다. 읽기는 같은 토큰으로 폴더 전체를 다룹니다.
- Source와 Core는 경로를 Vault 기준 상대 경로 문자열로 다룹니다. Drive adapter가 파일 ID, ETag, HTTP 처리를 담당합니다.
- `workspace.json` 등 UI 전용 상태는 Drive Vault가 아니라 브라우저의 `localStorage` 또는 `IndexedDB`에 저장합니다.

## 기능 동등성 현황

| 영역 | Desktop | 현재 Web | 다음 작업 |
|---|---|---|---|
| 디자인 토큰/치수 | `core::design` | 토큰 131개를 CSS 변수로 제공 | 공통 치수 내보내기 |
| 셸 | 왼쪽 도크, 탭, 뷰 헤더 | Files/Search 도크, 탭 바, 뷰 헤더 | 프레임 UI 일치 |
| 파일 | `FileTree`, 행 높이 25px | 같은 `FileTree`, 정렬/접기 | 드래그 |
| 읽기 | reading model | `markdown::render_html` + CSS | reading model 공유 |
| 편집 | Live Preview + Source | Source textarea, 저장/포커스 해제 | Live Preview |
| 검색 | 전체 검색 패널 | 파일명 및 색인된 전체 텍스트 | 검색 결과 미리보기/메타데이터 필터 |
| 설정 | Appearance + Editor | 동일한 단일 모달 | 창 배치 일치 |
| 빠른 전환 | GPUI 대화상자 | Ctrl+O 모달 | 행/키 동작 일치 |
| 파일 작업 | 컨텍스트 메뉴 | 새로 만들기/이름 변경/삭제/컨텍스트 메뉴 | 이동/복사 |
| Graph/Canvas/PDF | GPUI 뷰 | 미구현 | DOM 렌더러 |
| 창 UI | 네이티브 프레임 | 브라우저 UI | PWA/standalone |

## 단계

1. Drive 색인, 파일 트리, Markdown 읽기, Source 편집, 탭, 디자인 토큰 셸. **당시 기준 진행 중.**
2. Live Preview, reading model, 메뉴, 검색 패널, 설정, 빠른 전환.
3. Canvas, Graph, PDF, 내보내기, 오프라인 캐시.
4. Vercel 배포, 사용자 도메인, OAuth 허용 Origin, 성능 개선.

공유 application/domain crate는 프레임워크에 종속되지 않습니다. 이 Leptos 앱은 참고용으로 보존되어 있으며, 현재 GPUI Web 렌더러로 쓰이지 않습니다.

## Google Cloud 설정

아래 설정과 배포는 보존된 Leptos 배포를 기준으로 합니다.

1. Google Cloud project를 만들고 **Google Drive API**와 **Google Picker API**를 사용 설정합니다
   (APIs & Services → Library).
2. OAuth 동의 화면을 External로 구성합니다. Data Access에
   `https://www.googleapis.com/auth/drive`를 추가하고, 사용할 계정을 테스트 사용자로 등록합니다.
   `drive`는 restricted scope이므로 Testing 상태에서는 테스트 사용자로만 승인할 수 있습니다.
3. **Credentials → OAuth client ID → Web application**을 만듭니다.
4. 다음 **Authorized JavaScript origins**를 추가합니다.
   - `http://127.0.0.1:8080`
   - `https://tft-web-sage.vercel.app` (production)
5. `apps/web/config.example.js`를 `apps/web/config.js`로 복사하고 client ID를 채웁니다. API key는
   Picker에 선택 사항이므로 처음에는 비워 둡니다.
6. Trunk를 한 번 설치한 뒤 serve합니다.

   ```sh
   cargo install trunk --locked
   cd apps/web
   trunk serve
   ```

7. `http://127.0.0.1:8080`을 열고 로그인한 뒤 **Pick vault**를 누릅니다. Picker를 쓸 수 없으면
   fallback 입력란에 Drive 폴더 ID를 붙여 넣거나 **Create test vault**를 사용합니다.

`config.js`는 특정 Google project를 가리키므로 gitignore되어 있습니다. 이 파일에는 공개 식별자만
들어갑니다: OAuth client ID와, 필요하면 브라우저 API key입니다. OAuth **client secret**, refresh
token, 사용자 access token은 이 파일이나 다른 frontend 파일, 저장소 어디에도 넣지 마세요. 노출되었다면
Google Cloud Console에서 폐기하세요.

Picker 문제 해결(401, `API key not valid`, `access_denied`)은 `apps/web/README.md`에 있습니다.

## Vercel 배포

배포 대상은 Vercel의 `tft-web` project, production origin은 `https://tft-web-sage.vercel.app`입니다.

```powershell
cd apps/web
.\deploy.ps1                 # Trunk로 빌드하고, API key를 비우고, 배포
.\deploy.ps1 -WithApiKey     # Vercel origin으로 제한한 key에만 사용
```

배포 전에 Vercel origin이 OAuth client의 Authorized JavaScript origins에 있어야 합니다.
`vercel.json`이 CSP와 보안 헤더를 제공합니다. 배포되는 것은 공개 식별자뿐입니다: client secret,
token, vault 데이터는 없습니다. 파일은 사용자의 Drive에 남습니다.

## 보안

- `drive` scope로 Picker가 선택한 PKMS 폴더의 기존 노트를 읽고, 쓰고, 휴지통으로 옮깁니다.
- OAuth token은 메모리에만 보관하고 client secret은 사용하지 않습니다.
- 배포 bundle에 API key를 포함하지 않습니다.
- 브라우저에서 렌더링한 노트 HTML을 정화해 script, frame, form, `on*` 속성을 제거합니다.
- Vercel은 정적 파일만 제공합니다. Vault 데이터는 호스팅 서버로 전송되지 않습니다.
- CSP, HSTS, referrer 및 permissions policy는 `apps/web/vercel.json`에 있습니다.
