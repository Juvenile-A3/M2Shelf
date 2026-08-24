<p align="center">
  <img src="./src-tauri/icons/128x128.png" width="96" height="96" alt="M²Shelf Logo">
</p>

<h1 align="center">M²Shelf</h1>

<p align="center"><strong>MORI MEDIA SHELF</strong></p>

<p align="center"><a href="./README.md">简体中文</a> · <a href="./README.en-US.md">English</a> · <a href="./README.ja-JP.md">日本語</a></p>

<p align="center">전적으로 GPT로 제작된, Windows 로컬 미디어 컬렉션을 위한 로컬 우선·원본 미디어 읽기 전용 브라우저입니다.</p>

M²Shelf는 내장·외장 드라이브 또는 NAS 매핑 폴더에 저장된 애니메이션, 영화 및 관련 리소스를 별도 색인으로 관리하며 포스터 보기, Bangumi 메타데이터, 태그, 즐겨찾기, 시청 기록 및 외부 플레이어 실행 기능을 제공합니다.

쉽게 말해, 파일 탐색기에서 언어·인코딩 방식·자막 그룹 등이 뒤섞여 알아보기 어려운 긴 파일명으로 보이던 애니메이션 컬렉션을 아래와 같은 깔끔한 포스터 그리드로 한 번에 바꿔 줍니다.

<img width="1427" height="888" alt="M²Shelf 포스터 보기 예시" src="https://github.com/user-attachments/assets/c09f2e18-aef0-4e13-ae1a-b5a7825bc3dc" />

작품 상세 페이지에서는 원래 파일명을 그대로 확인할 수 있으며, 클릭 한 번으로 Windows 파일 탐색기에서 해당 위치를 열 수 있습니다.

**원본 미디어 파일을 이동, 삭제, 이름 변경 또는 수정하지 않으며 기존 폴더 구조를 다시 정리하도록 요구하지 않습니다.**

## 주요 기능

- 여러 미디어 라이브러리를 관리하고 원하는 깊이까지 폴더를 재귀적으로 스캔합니다.
- 모든 리소스, 개별 라이브러리 및 실제 폴더 계층을 포스터 또는 목록 형태로 탐색합니다.
- 로컬 이름, 파일 이름, Bangumi 다국어 제목 및 사용자 태그를 검색합니다.
- 작품, 시리즈 및 기타 리소스를 자동 분류하고 수동 분류를 유지합니다.
- 신뢰도가 높은 Bangumi 항목을 자동 매칭하며 수동 검색, 수정 및 표지 재시도를 지원합니다.
- 동영상과 함께 자막, 이미지, 오디오, 문서, 압축 파일 등의 관련 리소스를 표시합니다.
- 설정한 외부 플레이어로 재생하고 Windows 파일 탐색기에서 파일 위치를 엽니다.
- 태그, 한 단계로 구성된 이름 지정 즐겨찾기 폴더 및 편집 모드로 일괄 정리합니다.
- M²Shelf가 재생을 성공적으로 시작한 작품을 최신순으로 표시합니다.
- 简体中文, English, 日本語, 한국어를 지원합니다.
- 시스템 설정, 라이트 및 다크 테마를 지원합니다.
- 창 크기, 정렬 선택 및 각 탐색 영역의 세션 내 위치를 기억합니다.
- 앱 전용 표지 캐시 위치 사용자 지정.

## 로컬 우선 설계와 개인정보 보호

미디어 폴더는 항상 읽기 전용으로 취급됩니다. M²Shelf의 색인, 표시 이름, Bangumi 연결 정보, 태그, 즐겨찾기, 시청 기록 및 설정은 앱 전용 SQLite 데이터베이스에 저장되며 표지는 앱 캐시에 저장됩니다.

Bangumi 검색과 표지 다운로드에는 인터넷 연결이 필요합니다. 로컬 색인 탐색과 로컬 파일 열기는 Bangumi에 의존하지 않습니다. 미디어 서버나 클라우드 계정이 필요하지 않으며 미디어 파일을 원격 서비스에 업로드하지 않습니다.

## 다운로드

현재 버전: **M²Shelf 0.5.7** (Windows x64)

- [Portable 버전 다운로드](https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/M2Shelf-Portable-0.5.7-x64.zip)
- [최신 Release 보기](https://github.com/Undermori/M2Shelf/releases/latest)
- [모든 Release 보기](https://github.com/Undermori/M2Shelf/releases)

Portable 버전 사용 방법:

1. ZIP 전체를 압축 해제하고 압축 파일 내부에서 직접 실행하지 마세요.
2. `M2Shelf.exe`를 두 번 클릭하세요.
3. 미디어 폴더를 추가하고 스캔하세요.
4. 필요한 경우 외부 플레이어 경로를 설정하세요.

Portable은 앱 자체를 설치할 필요가 없다는 뜻입니다. 데이터베이스, 설정 및 기본 표지 캐시는 Windows 앱 데이터 폴더에 저장됩니다. 현재 빌드는 코드 서명되지 않았으므로 Windows SmartScreen에서 ‘알 수 없는 게시자’ 경고가 표시될 수 있습니다. 화면을 표시하려면 Microsoft Edge WebView2 Runtime이 필요합니다.

## 현재 범위

M²Shelf는 현재 내장 플레이어, 온라인 스트리밍, 트랜스코딩, 미디어 서버, 계정 동기화, 자동 자막 또는 이어보기 진행률을 제공하지 않습니다. 미디어 파일을 자동으로 이동하거나 이름을 변경하지도 않습니다.

## 개발

기술 스택: Tauri 2, Rust, React 19, TypeScript, Vite 및 SQLite.

```powershell
npm install
npm run tauri dev
```

커밋 전 확인:

```powershell
npm run typecheck
npm run build
npm run validate
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

Windows용 공식 결과물은 `scripts/build_windows_release.ps1`로 빌드합니다.

## 프로젝트 문서

- [개발 규칙](./AGENTS.md)
- [제품 사양](./docs/PRODUCT_SPEC.md)
- [현재 구현](./docs/PROJECT_CONTEXT.md)
- [장기 결정 사항](./docs/DECISIONS.md)
- [빠른 프로젝트 안내](./PROJECT_DOCUMENTATION.md)

## 제작자

- [森下Undermori · Bilibili](https://space.bilibili.com/2903441)
- [Undermori · X](https://x.com/f_undermori)
