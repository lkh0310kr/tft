---
aliases:
  - Workspace Refactor
tags:
  - tft/development
---
# Workspace 리팩터링

## 목표

`apps/desktop/src/workspace.rs`와 하위 모듈의 책임 및 상태 소유를 명확히 한다. Desktop 전용 기능은 Desktop 앱에 두고, Desktop과 Web이 같은 의미와 결과를 갖는 데이터 및 전이만 공유 계층에 둔다. 파일을 나누거나 코드 줄 수를 줄이는 것 자체는 목표가 아니다.

## 작업 원칙

- 한 번에 한 책임 경계만 변경한다.
- 코드 이동 전 호출자, 상태의 reader/writer, GPUI·OS·filesystem 의존성 및 Web 대응 동작을 확인한다.
- 기존 기능과 Vault 저장 형식을 유지한다.
- 기존 `crates/app` 및 `crates/domain` API를 먼저 확인하고, 실제 양쪽 호출부의 결과가 같을 때만 공통화한다.
- 단순 forwarding wrapper, 중복 UI controller 및 Desktop 구조의 무조건적인 Web 복제를 만들지 않는다.
- 테스트는 별도 요청 또는 검증 계획에 포함된 경우에만 실행한다.

## 현재 구조

`Workspace`는 Desktop GPUI 상태와 플랫폼 동작을 소유하고 화면 및 feature를 조합한다. 구현은 `apps/desktop/src/workspace/` 아래 기능별 모듈로 분리돼 있다. Web은 별도의 Workspace UI를 사용하며 공유 모델과 순수 전이를 호출한다.

## 남은 작업

### 1. 상태 소유 조사

- [ ] `Workspace`의 남은 필드와 메서드별 reader/writer, 변경 주체, 생명주기 및 플랫폼 의존성을 정리한다.
- [ ] 이미 feature state로 이동한 상태는 현재 코드와 대조해 단일 소유자와 접근 경계를 확인한다.
- [ ] 각 분리 후보의 Desktop 호출부와 Web 대응 동작을 추적한다.

### 2. Desktop 기능 경계

- [ ] Workspace root에 남은 상태 및 메서드가 조합자 역할에 필요한지 확인한다.
- [ ] 모듈별 책임이 겹치거나 이름과 실제 책임이 어긋나는 곳을 확인한다.
- [ ] 불필요한 공개 API, 미사용 메서드/import, 중복 상태 및 낡은 주석을 점검한다.
- [ ] 상태 캡슐화가 필요한 경우 실제 불변 조건과 여러 호출자의 요구를 확인한 뒤 최소 API로 조정한다.

### 3. Desktop/Web 공통 경계

- [ ] 문서 편집 결과와 저장 요청의 데이터 계약을 양쪽 구현에서 추적한다.
- [ ] local Vault watcher·filesystem과 Drive·browser 인증을 플랫폼 효과로 분리하고, 공통화 가능한 순수 동작이 있는지 확인한다.
- [ ] 이미 공유된 API마다 Desktop/Web 호출부와 결과가 맞는지 확인한다.
- [ ] 공통화하지 않는 후보는 구현 차이와 현재 공유하지 않는 이유를 기록한다.

### 4. 마감 검증

- [ ] 변경된 모듈을 포맷하고 영향 범위에 맞춰 Desktop 및 Web 빌드를 확인한다.
- [ ] 테스트는 요청되거나 검증 계획에 명시된 경우에만 실행한다.
- [ ] 계획과 실제 코드가 일치하고, 검증하지 않은 동작이 완료로 기록되지 않았는지 확인한다.

## 완료 기준

- Workspace root는 Desktop 상태 소유자이자 UI 조합자로 역할이 분명하다.
- 각 하위 모듈의 책임과 상태 흐름을 코드에서 추적할 수 있다.
- 공통 API는 양쪽 호출부와 결과가 확인된 동작에 한정된다.
- Desktop 전용 효과와 공통 데이터·전이의 소유 계층이 명확하다.
- 요구된 검증을 마쳤고, 문서가 실제 코드 및 검증 결과와 일치한다.

## 다음 작업

1번 상태 소유 조사부터 이어간다. 필드 단위로 reader/writer와 생명주기를 확인하고, 근거가 확보된 경우에만 코드를 변경한다.
