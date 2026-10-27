# IoT-Plattform

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Projekt-Maskottchen" />
</p>

Eine All-in-one-SaaS-IoT-Plattform für die einheitliche Anbindung führender Gerätehersteller im In- und Ausland (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT usw.) mit Geräteverwaltung, Dingmodell, Regeln & Alarmen, Zeitreihendaten, CDN-Auslieferung und Multiplattform-Apps. Das Backend ist ein Rust-Mikroservice-Workspace (e-cat), die Frontends sind Flutter + HarmonyOS, mit Unterstützung für 13 Sprachen.

## Funktionen

- **Multi-Vendor-Anbindung**: Cloud-to-Cloud-OAuth (Tuya / Xiaomi / Huawei / AWS / Azure) + direktes MQTT (mTLS)
- **Dingmodell**: einheitliche Modellierung von Eigenschaft / Ereignis / Dienst — im Admin modelliert, im Client dynamisch gerendert
- **Gerätelebenszyklus**: Registrieren → Aktivieren / Deaktivieren → Entbinden → Löschen, mit Online-/Offline-Status zur Laufzeit
- **Regeln & Alarme**: Schwellwertregeln, Szenario-Automatisierung, Echtzeit-Push per WebSocket
- **Daten & Berichte**: TDengine-Zeitreihenspeicher, Verlaufskurven, CSV/Excel-Export
- **CDN-Verwaltung**: Anbieterkonfiguration, Aktivieren / Deaktivieren, Refresh & Prewarm, signierte URLs
- **Multi-Tenant-SaaS**: Tenant-Isolation, Kontingente, Rollen (admin / operator / read-only)
- **Sicherheit**: Security-rust-Inbound-Scanning, JWT + RBAC, gegenseitiges TLS, AES-verschlüsselte Anmeldedaten, Ratenbegrenzung & Circuit Breaker
- **i18n in 13 Sprachen**, Flutter Web / Mobile und natives HarmonyOS (4 Apps)

## Architektur

![Architektur](../architecture.de.svg)

## Geräte-Anbindungsabläufe

![Flussdiagramm](../flow.de.svg)

## Funktionsübersicht

![Funktionsübersicht](../features.de.svg)

## Gerätelebenszyklus

![Lebenszyklus](../lifecycle.de.svg)

## Sicherheitsarchitektur

![Sicherheit](../security.de.svg)

## Technologie-Stack

| Ebene | Technologie |
|----|------|
| Frontend | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Backend | Rust (axum · tokio · gRPC) · Security-rust-Scanning |
| Middleware | EMQX (MQTT) · Kafka (Event-Bus) · gRPC (internes RPC) |
| Speicher | MySQL 8 (Metadaten) · TDengine (Zeitreihen) · Redis (Shadow / Cache) · S3 / MinIO (Objekte) |
| Anbindung | Cloud-to-Cloud-OAuth-Adapter · direktes MQTT (mTLS) |

## Repository-Struktur

```
├── apps/            # Frontend-Apps
│   ├── admin/       # Admin-Konsole (Flutter + HarmonyOS)
│   └── client/      # Client-App (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # Doku, Diagramme, Spendenbilder
├── scripts/         # Build- / Validierungs- / Smoke-Test-Skripte
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## Ein-Klick-Installation

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

Das Skript erledigt automatisch: Infrastruktur starten (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → 6 Geschäftsdienste bauen → .env-Konfiguration erzeugen → Dienstliste und Startbefehle ausgeben. Wiederholtes Ausführen ist sicher.

## Installationsanleitung

### Voraussetzungen

- Docker 24+ und docker compose (oder docker-compose)
- Rust 1.80+ (stable, zum Kompilieren der Dienste; das Skript baut automatisch, wenn cargo installiert ist)
- Portprüfung: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 müssen frei sein

## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.

## Verwendung

### Anmeldung

- Admin-Konsole: melden Sie sich mit `admin / admin123` (Mandant `tenant-1`) an

### Dienstports

| Dienst | Port |
|------|------|
| iot-gateway (Gateway / öffentliche API) | 8080 |
| iot-device (Gerätedienst) | 8081 |
| iot-access (Zugang / Auth) | 8082 |
| iot-data (Datendienst) | 8083 |
| iot-rule (Regel-Engine) | 8084 |
| iot-cdn (CDN-Verwaltung) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Modulnutzung

- **Geräteverwaltung**: Gerät in der Konsole hinzufügen → Vendor-OAuth oder direkten MQTT wählen; Online-Status und Lebenszyklus in den Details prüfen
- **Dingmodell**: Attribute / Ereignisse / Dienste für Gerätekategorien definieren; das Client-Bedienfeld rendert automatisch
- **Regeln & Alarme**: Schwellwertregeln und Szenario-Automatisierung konfigurieren; Echtzeit-Push über WebSocket bei Auslösung
- **Verlaufsdaten**: iot-data speichert Zeitreihen; Verlaufskurven ansehen und als CSV / Excel exportieren
- **Berichte & Statistik**: mehrdimensionale Berichte über Geräte / Daten / CDN / Alarme / Mandanten
- **CDN-Verwaltung**: Multi-Vendor-CDN konfigurieren, Refresh / Prewarm und signierte URLs
- **Multi-Vendor-Zugang**: Cloud-zu-Cloud-OAuth-Adapter für Tuya / Xiaomi / Huawei / AWS / Azure

## Umsetzungsphasen

| Phase | Umfang |
|------|------|
| P0 Gerüst | Repo-Struktur, iot-gateway + iot-device + Docker-Orchestrierung (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Anbindung | iot-access + Tuya-Adapter + direktes MQTT |
| P2 Daten | iot-data + TDengine + Verlaufskurven |
| P3 Regeln | iot-rule-Alarme + Szenario-Automatisierung |
| P4 Multi-Vendor + CDN | Xiaomi/Huawei/AWS/Azure-Adapter + iot-cdn |
| P5 Frontend | apps/admin + apps/client komplett |
| P6 Launch | Sicherheitshärtung, Lasttests, OTA |

## Unterstützung & Spenden

Ihre Unterstützung treibt das Projekt voran. Spenden sind herzlich willkommen — vielen Dank!

### Per QR-Code spenden (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="WeChat-Pay-QR-Code" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="Alipay-QR-Code" title="Alipay" />
</p>

WeChat Pay · Alipay

## Lizenz

Der Code dieses Projekts dient ausschließlich zu Lern- und Austauschzwecken.
