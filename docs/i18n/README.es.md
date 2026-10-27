# Plataforma IoT

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Mascota del proyecto" />
</p>

Una plataforma SaaS de IoT todo en uno que unifica el acceso a los principales fabricantes de dispositivos nacionales e internacionales (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT, etc.), ofreciendo gestión de dispositivos, modelos de cosas, reglas y alertas, datos de series temporales, entrega CDN y aplicaciones multiplataforma. El backend es un workspace de microservicios Rust (e-cat), los frontends son Flutter + HarmonyOS, con soporte para 13 idiomas.

## Características

- **Acceso multi-fabricante**: OAuth nube a nube (Tuya / Xiaomi / Huawei / AWS / Azure) + MQTT directo (mTLS)
- **Modelo de cosas**: modelado unificado de propiedad / evento / servicio — modelado en el admin, renderizado dinámicamente en el cliente
- **Ciclo de vida del dispositivo**: registro → habilitar / deshabilitar → desvincular → eliminar, con estado en línea / fuera de línea en tiempo de ejecución
- **Reglas y alertas**: reglas de umbral, automatización de escenas, push WebSocket en tiempo real
- **Datos e informes**: almacenamiento de series temporales TDengine, curvas históricas, exportación CSV/Excel
- **Gestión de CDN**: configuración de proveedores, habilitar / deshabilitar, refresco y precalentamiento, URL firmadas
- **SaaS multi-tenant**: aislamiento de inquilinos, cuotas, roles (admin / operator / read-only)
- **Seguridad**: escaneo entrante security-rust, JWT + RBAC, TLS mutuo, credenciales cifradas con AES, limitación de tasa y disyuntor
- **i18n en 13 idiomas**, Flutter Web / Mobile y HarmonyOS nativo (4 aplicaciones)

## Arquitectura

![Arquitectura](../architecture.es.svg)

## Flujos de acceso de dispositivos

![Diagrama de flujo](../flow.es.svg)

## Mapa de funciones

![Mapa de funciones](../features.es.svg)

## Ciclo de vida del dispositivo

![Ciclo de vida](../lifecycle.es.svg)

## Arquitectura de seguridad

![Seguridad](../security.es.svg)

## Pila tecnológica

| Capa | Tecnología |
|----|------|
| Frontend | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Backend | Rust (axum · tokio · gRPC) · escaneo security-rust |
| Middleware | EMQX (MQTT) · Kafka (bus de eventos) · gRPC (RPC interno) |
| Almacenamiento | MySQL 8 (metadatos) · TDengine (series temporales) · Redis (shadow / caché) · S3 / MinIO (objetos) |
| Acceso | Adaptadores OAuth nube a nube · MQTT directo (mTLS) |

## Estructura del repositorio

```
├── apps/            # Aplicaciones frontend
│   ├── admin/       # Consola de administración (Flutter + HarmonyOS)
│   └── client/      # Aplicación cliente (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # Documentación, diagramas, imágenes de donaciones
├── scripts/         # Scripts de build / validación / smoke tests
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## Instalación en un clic

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

El script hace automáticamente: levantar la infraestructura (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → compilar los 6 servicios de negocio → generar la configuración .env → imprimir la lista de servicios y los comandos de arranque. Es seguro ejecutarlo varias veces.

## Guía de instalación

### Requisitos previos

- Docker 24+ y docker compose (o docker-compose)
- Rust 1.80+ (stable, para compilar los servicios; el script compila automáticamente si cargo está instalado)
- Comprobación de puertos: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 deben estar libres

### Pasos de instalación

1. Instalar y arrancar la infraestructura: `./scripts/install.sh` ejecuta `docker compose up -d`
2. Compilar los servicios: el script ejecuta automáticamente `cargo build --release` si detecta cargo; los binarios se copian a `scripts/bin/` (o usa los de `e-cat/target/release/`)
3. Arrancar los servicios: lanza los 6 servicios con los comandos impresos al final del script
4. Migración de base de datos: se ejecuta automáticamente al arrancar; en el primer inicio iot-access crea el inquilino por defecto y la cuenta de administrador

## Uso

### Inicio de sesión

- Consola de administración: entra con la cuenta `admin / admin123` (inquilino `tenant-1`)

### Puertos de los servicios

| Servicio | Puerto |
|------|------|
| iot-gateway (pasarela / API pública) | 8080 |
| iot-device (servicio de dispositivos) | 8081 |
| iot-access (conexión / autenticación) | 8082 |
| iot-data (servicio de datos) | 8083 |
| iot-rule (motor de reglas) | 8084 |
| iot-cdn (gestión de CDN) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Uso de los módulos

- **Gestión de dispositivos**: añade un dispositivo en la consola → elige OAuth del fabricante o MQTT directo; estado en línea y ciclo de vida en los detalles
- **Modelo de cosas**: define propiedad / evento / servicio para las categorías; el panel del cliente se renderiza automáticamente
- **Reglas y alertas**: configura reglas de umbral y automatización de escenarios; notificación WebSocket en tiempo real
- **Datos históricos**: iot-data almacena series temporales; curvas históricas y exportación CSV / Excel
- **Informes y estadísticas**: informes multidimensionales de dispositivos / datos / CDN / alertas / inquilinos
- **Gestión de CDN**: configura CDN multi-fabricante, refresco / prewarm y URL firmadas
- **Acceso multi-fabricante**: adaptadores OAuth nube a nube Tuya / Xiaomi / Huawei / AWS / Azure

## Fases de implementación

| Fase | Alcance |
|------|------|
| P0 Esqueleto | Estructura del repo, iot-gateway + iot-device + orquestación Docker (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Acceso | iot-access + adaptador Tuya + MQTT directo |
| P2 Datos | iot-data + TDengine + curvas históricas |
| P3 Reglas | alertas iot-rule + automatización de escenas |
| P4 Multi-fabricante + CDN | Adaptadores Xiaomi/Huawei/AWS/Azure + iot-cdn |
| P5 Frontend | apps/admin + apps/client completos |
| P6 Lanzamiento | Endurecimiento de seguridad, pruebas de carga, OTA |

## Soporte y donaciones

Su apoyo impulsa el proyecto. ¡Las donaciones son bienvenidas, muchísimas gracias!

### Donar escaneando el código QR (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="Código QR de WeChat Pay" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="Código QR de Alipay" title="Alipay" />
</p>

WeChat Pay · Alipay

## Licencia

El código de este proyecto se proporciona únicamente con fines de aprendizaje e intercambio.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
