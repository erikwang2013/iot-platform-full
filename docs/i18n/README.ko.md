# IoT 플랫폼

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="프로젝트 마스코트" />
</p>

국내외 주요 기기 제조사(투야, 샤오미, 화웨이, AWS IoT, Azure IoT 등)를 통합 연결하는 원스톱 SaaS IoT 플랫폼으로, 기기 관리, 사물 모델, 규칙 및 알림, 시계열 데이터, CDN 배포, 멀티 플랫폼 앱을 제공합니다. 백엔드는 Rust 마이크로서비스(e-cat 워크스페이스), 프론트엔드는 Flutter + HarmonyOS이며 13개 언어를 지원합니다.

## 주요 기능

- **다중 제조사 연동**: 클라우드 간 OAuth 연동(투야 / 샤오미 / 화웨이 / AWS / Azure) + 직접 MQTT(mTLS)
- **사물 모델**: 속성 / 이벤트 / 서비스 통합 모델링 — 관리자에서 모델링, 클라이언트에서 동적 렌더링
- **기기 수명 주기**: 등록 → 활성화 / 비활성화 → 바인딩 해제 → 삭제, 런타임 온라인 / 오프라인 상태
- **규칙 및 알림**: 임계값 규칙, 시나리오 자동화, WebSocket 실시간 푸시
- **데이터 및 보고서**: TDengine 시계열 저장, 이력 곡선, CSV/Excel 내보내기
- **CDN 관리**: 공급업체 설정, 활성화 / 비활성화, 새로고침 및 워밍업, 서명 URL
- **멀티 테넌트 SaaS**: 테넌트 격리, 할당량, 역할 권한(admin / operator / read-only)
- **보안**: security-rust 인바운드 스캔, JWT + RBAC, 상호 TLS, 자격 증명 AES 암호화, 속도 제한 및 회로 차단기
- **13개 언어 i18n**, Flutter Web / Mobile 및 네이티브 HarmonyOS(4개 앱)

## 아키텍처

![아키텍처](../architecture.ko.svg)

## 기기 연동 흐름

![흐름도](../flow.ko.svg)

## 기능 맵

![기능 맵](../features.ko.svg)

## 수명 주기

![수명 주기](../lifecycle.ko.svg)

## 보안 아키텍처

![보안 아키텍처](../security.ko.svg)

## 기술 스택

| 계층 | 기술 |
|----|------|
| 프론트엔드 | Flutter(Web / Mobile) · HarmonyOS ArkTS |
| 백엔드 | Rust(axum · tokio · gRPC) · security-rust 스캔 |
| 미들웨어 | EMQX(MQTT) · Kafka(이벤트 버스) · gRPC(내부 RPC) |
| 저장소 | MySQL 8(메타데이터) · TDengine(시계열) · Redis(섀도우 / 캐시) · S3 / MinIO(객체) |
| 연동 | 클라우드 간 OAuth 어댑터 · 직접 MQTT(mTLS) |

## 저장소 구조

```
├── apps/            # 프론트엔드 앱
│   ├── admin/       # 관리자 콘솔(Flutter + HarmonyOS)
│   └── client/      # 클라이언트 앱(Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # 문서, 다이어그램, 후원 이미지
├── scripts/         # 빌드 / 검증 / 스모크 테스트 스크립트
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## 원클릭 설치

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

스크립트가 자동으로: 인프라 기동(MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → 6개 비즈니스 서비스 빌드 → .env 설정 생성 → 서비스 목록과 시작 명령 출력. 반복 실행해도 안전합니다.

## 설치 안내

### 사전 요구 사항

- Docker 24+ 및 docker compose(또는 docker-compose)
- Rust 1.80+(stable, 서비스 컴파일용. cargo가 설치되어 있으면 스크립트가 자동 빌드)
- 포트 점유 확인: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041이 비어 있어야 함

### 설치 단계

1. 인프라 설치 및 시작: `./scripts/install.sh`가 `docker compose up -d`를 실행합니다
2. 서비스 빌드: cargo가 감지되면 자동으로 `cargo build --release` 실행, 바이너리는 `scripts/bin/`에 출력(`e-cat/target/release/` 결과물 직접 사용 가능)
3. 서비스 시작: 스크립트 끝에 출력된 명령으로 6개 서비스를 하나씩 시작
4. DB 마이그레이션: 서비스 시작 시 자동 실행, 수동 작업 불필요. 첫 실행 시 iot-access가 기본 테넌트와 관리자 계정을 생성

## 사용 방법

### 로그인

- 관리자 콘솔: 기본 계정 `admin / admin123`(테넌트 `tenant-1`)으로 로그인

### 서비스 포트

| 서비스 | 포트 |
|------|------|
| iot-gateway(게이트웨이 / 공개 API) | 8080 |
| iot-device(디바이스 서비스) | 8081 |
| iot-access(접속 / 인증) | 8082 |
| iot-data(데이터 서비스) | 8083 |
| iot-rule(규칙 엔진) | 8084 |
| iot-cdn(CDN 관리) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### 모듈 사용법

- **디바이스 관리**: 관리자 콘솔에서 디바이스 추가 → 벤더 OAuth 또는 직접 MQTT 선택, 상세에서 온라인 상태와 라이프사이클 확인
- **사물 모델**: 디바이스 카테고리에 속성 / 이벤트 / 서비스 정의, 클라이언트 제어판이 자동 렌더링
- **규칙 및 알림**: 임계값 규칙과 시나리오 자동화 설정, 트리거 시 WebSocket으로 실시간 푸시
- **이력 데이터**: iot-data가 시계열 데이터 저장, 이력 곡선 조회 및 CSV / Excel 내보내기
- **보고서 및 통계**: 디바이스 / 데이터 / CDN / 알림 / 테넌트 다차원 보고서
- **CDN 관리**: 멀티 벤더 CDN 설정, 리프레시 / 프리웜 및 서명 URL 지원
- **멀티 벤더 접속**: Tuya / Xiaomi / Huawei / AWS / Azure 클라우드 간 OAuth 어댑터

## 구현 단계

| 단계 | 범위 |
|------|------|
| P0 골격 | 저장소 구조, iot-gateway + iot-device + Docker 오케스트레이션(MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 연동 | iot-access + 투야 어댑터 + 직접 MQTT |
| P2 데이터 | iot-data + TDengine + 이력 곡선 |
| P3 규칙 | iot-rule 알림 + 시나리오 자동화 |
| P4 다중 제조사 + CDN | 샤오미/화웨이/AWS/Azure 어댑터 + iot-cdn |
| P5 프론트엔드 | apps/admin + apps/client 전체 |
| P6 출시 | 보안 강화, 부하 테스트, OTA |

## 지원 및 후원

여러분의 지원은 프로젝트가 지속 발전하는 원동력입니다. 후원해 주시면 정말 감사하겠습니다!

### QR 코드 후원(WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="WeChat Pay QR 코드" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="Alipay QR 코드" title="Alipay" />
</p>

WeChat Pay · Alipay

## 라이선스

이 프로젝트의 코드는 학습 및 교류 목적으로만 제공됩니다.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
