# Plateforme IoT

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Mascotte du projet" />
</p>

Une plateforme SaaS IoT tout-en-un qui unifie l'accès aux grands fabricants d'appareils nationaux et internationaux (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT, etc.), offrant gestion des appareils, modèles de choses, règles et alertes, données de séries temporelles, diffusion CDN et applications multiplateformes. Le backend est un workspace de microservices Rust (e-cat), les frontends sont Flutter + HarmonyOS, avec prise en charge de 13 langues.

## Fonctionnalités

- **Accès multi-fabricants** : OAuth cloud-à-cloud (Tuya / Xiaomi / Huawei / AWS / Azure) + MQTT direct (mTLS)
- **Modèle de choses** : modélisation unifiée des propriétés / événements / services — modélisé dans l'admin, rendu dynamiquement dans le client
- **Cycle de vie des appareils** : inscription → activation / désactivation → dissociation → suppression, avec statut en ligne / hors ligne à l'exécution
- **Règles et alertes** : règles de seuil, automatisation de scénarios, push WebSocket en temps réel
- **Données et rapports** : stockage de séries temporelles TDengine, courbes d'historique, export CSV/Excel
- **Gestion CDN** : configuration des fournisseurs, activation / désactivation, rafraîchissement et préchauffage, URL signées
- **SaaS multi-tenant** : isolation des locataires, quotas, rôles (admin / operator / read-only)
- **Sécurité** : analyse entrante security-rust, JWT + RBAC, TLS mutuel, identifiants chiffrés AES, limitation de débit et disjoncteur
- **i18n en 13 langues**, Flutter Web / Mobile et HarmonyOS natif (4 applications)

## Architecture

![Architecture](../architecture.fr.svg)

## Flux d'accès des appareils

![Diagramme de flux](../flow.fr.svg)

## Carte des fonctionnalités

![Carte des fonctionnalités](../features.fr.svg)

## Cycle de vie des appareils

![Cycle de vie](../lifecycle.fr.svg)

## Architecture de sécurité

![Sécurité](../security.fr.svg)

## Pile technologique

| Couche | Technologie |
|----|------|
| Frontend | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Backend | Rust (axum · tokio · gRPC) · analyse security-rust |
| Middleware | EMQX (MQTT) · Kafka (bus d'événements) · gRPC (RPC interne) |
| Stockage | MySQL 8 (métadonnées) · TDengine (séries temporelles) · Redis (shadow / cache) · S3 / MinIO (objets) |
| Accès | Adaptateurs OAuth cloud-à-cloud · MQTT direct (mTLS) |

## Structure du dépôt

```
├── apps/            # Applications frontend
│   ├── admin/       # Console d'administration (Flutter + HarmonyOS)
│   └── client/      # Application client (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # Docs, diagrammes, images de dons
├── scripts/         # Scripts de build / validation / smoke tests
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## Installation en une commande

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

Le script réalise automatiquement : démarrage de l'infrastructure (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → compilation des 6 services métier → génération de la configuration .env → affichage de la liste des services et des commandes de démarrage. L'exécution répétée est sans risque.

## Guide d'installation

### Prérequis

- Docker 24+ et docker compose (ou docker-compose)
- Rust 1.80+ (stable, pour compiler les services ; le script compile automatiquement si cargo est installé)
- Vérification des ports : 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 doivent être libres

### Étapes d'installation

1. Installer et démarrer l'infrastructure : `./scripts/install.sh` exécute `docker compose up -d`
2. Compiler les services : le script lance automatiquement `cargo build --release` si cargo est détecté, les binaires sont placés dans `scripts/bin/` (ou utilisez ceux de `e-cat/target/release/`)
3. Démarrer les services : lancez les 6 services avec les commandes affichées à la fin du script
4. Migration de la base : exécutée automatiquement au démarrage ; au premier lancement, iot-access crée le locataire par défaut et le compte administrateur

## Utilisation

### Connexion

- Console d'administration : connectez-vous avec `admin / admin123` (locataire `tenant-1`)

### Ports des services

| Service | Port |
|------|------|
| iot-gateway (passerelle / API publique) | 8080 |
| iot-device (service appareils) | 8081 |
| iot-access (connexion / authentification) | 8082 |
| iot-data (service données) | 8083 |
| iot-rule (moteur de règles) | 8084 |
| iot-cdn (gestion CDN) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Utilisation des modules

- **Gestion des appareils** : ajoutez un appareil dans la console → choisissez OAuth du fabricant ou MQTT direct ; statut en ligne et cycle de vie dans les détails
- **Modèle d'objet** : définissez propriété / événement / service pour les catégories d'appareils ; le panneau client est rendu automatiquement
- **Règles et alertes** : configurez des règles à seuil et l'automatisation de scénarios ; notification WebSocket en temps réel
- **Données historiques** : iot-data stocke les séries temporelles ; courbes d'historique et export CSV / Excel
- **Rapports et statistiques** : rapports multidimensionnels appareils / données / CDN / alertes / locataires
- **Gestion CDN** : configurez le CDN multi-fabricants, refresh / prewarm et URL signées
- **Accès multi-fabricants** : adaptateurs OAuth cloud à cloud Tuya / Xiaomi / Huawei / AWS / Azure

## Phases de mise en œuvre

| Phase | Périmètre |
|------|------|
| P0 Squelette | Structure du dépôt, iot-gateway + iot-device + orchestration Docker (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Accès | iot-access + adaptateur Tuya + MQTT direct |
| P2 Données | iot-data + TDengine + courbes d'historique |
| P3 Règles | alertes iot-rule + automatisation de scénarios |
| P4 Multi-fabricants + CDN | Adaptateurs Xiaomi/Huawei/AWS/Azure + iot-cdn |
| P5 Frontend | apps/admin + apps/client complets |
| P6 Lancement | Durcissement de la sécurité, tests de charge, OTA |

## Soutien et dons

Votre soutien fait avancer le projet. Les dons sont les bienvenus, merci infiniment !

### Donner par QR code (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="QR code WeChat Pay" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="QR code Alipay" title="Alipay" />
</p>

WeChat Pay · Alipay

## Licence

Le code de ce projet est fourni uniquement à des fins d'apprentissage et d'échange.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
