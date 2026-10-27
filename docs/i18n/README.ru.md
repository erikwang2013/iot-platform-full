# IoT Платформа

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Талисман проекта" />
</p>

Единая SaaS-платформа Интернета вещей, объединяющая ведущих производителей устройств (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT и др.), с управлением устройствами, моделями вещей, правилами и оповещениями, временными рядами, CDN-доставкой и мультиплатформенными приложениями. Бэкенд — микросервисы на Rust (рабочая область e-cat), фронтенд — Flutter + HarmonyOS, поддержка 13 языков.

## Возможности

- **Подключение вендоров**: OAuth cloud-to-cloud (Tuya / Xiaomi / Huawei / AWS / Azure) + прямой MQTT (mTLS)
- **Модель вещей**: единое моделирование свойств / событий / сервисов — моделирование в админке, динамический рендеринг в клиенте
- **Жизненный цикл устройства**: регистрация → включение / отключение → отвязка → удаление, статус онлайн / офлайн во время работы
- **Правила и оповещения**: пороговые правила, автоматизация сценариев, push через WebSocket в реальном времени
- **Данные и отчёты**: хранение временных рядов в TDengine, исторические кривые, экспорт CSV/Excel
- **Управление CDN**: конфигурация вендора, включение / отключение, обновление и прогрев, подписанные URL
- **Мультитенантный SaaS**: изоляция тенантов, квоты, роли (admin / operator / read-only)
- **Безопасность**: входящее сканирование security-rust, JWT + RBAC, взаимный TLS, шифрование учётных данных AES, лимиты и автоматический выключатель
- **i18n на 13 языках**, Flutter Web / Mobile и нативные HarmonyOS (4 приложения)

## Архитектура

![Архитектура](../architecture.ru.svg)

## Схемы подключения устройств

![Диаграмма потока](../flow.ru.svg)

## Карта функций

![Карта функций](../features.ru.svg)

## Жизненный цикл

![Жизненный цикл](../lifecycle.ru.svg)

## Архитектура безопасности

![Безопасность](../security.ru.svg)

## Технологический стек

| Слой | Технология |
|----|------|
| Фронтенд | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Бэкенд | Rust (axum · tokio · gRPC) · сканирование security-rust |
| Промежуточный слой | EMQX (MQTT) · Kafka (шина событий) · gRPC (внутренний RPC) |
| Хранилище | MySQL 8 (метаданные) · TDengine (временные ряды) · Redis (тень / кэш) · S3 / MinIO (объекты) |
| Подключение | Адаптеры OAuth cloud-to-cloud · прямой MQTT (mTLS) |

## Структура репозитория

```
├── apps/            # Фронтенд-приложения
│   ├── admin/       # Консоль администратора (Flutter + HarmonyOS)
│   └── client/      # Клиентское приложение (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # Документация, диаграммы, изображения для донатов
├── scripts/         # Скрипты сборки / проверки / смоук-тестов
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## Быстрая установка

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

Скрипт автоматически: поднимает инфраструктуру (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → собирает 6 бизнес-сервисов → генерирует конфигурацию .env → выводит список сервисов и команды запуска. Повторный запуск безопасен.

## Инструкция по установке

### Предварительные требования

- Docker 24+ и docker compose (или docker-compose)
- Rust 1.80+ (stable, для сборки сервисов; скрипт собирает автоматически, если установлен cargo)
- Проверка занятости портов: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 должны быть свободны

### Шаги установки

1. Установите и запустите инфраструктуру: `./scripts/install.sh` выполнит `docker compose up -d`
2. Сборка сервисов: при наличии cargo скрипт автоматически запускает `cargo build --release`, бинарники помещаются в `scripts/bin/` (или используйте артефакты из `e-cat/target/release/`)
3. Запуск сервисов: запустите 6 сервисов по командам, выведенным в конце скрипта
4. Миграция БД: выполняется автоматически при запуске сервисов; при первом старте iot-access создаёт тенант и учётную запись администратора

## Использование

### Вход

- Консоль администратора: войдите с учётной записью `admin / admin123` (тенант `tenant-1`)

### Порты сервисов

| Сервис | Порт |
|------|------|
| iot-gateway (шлюз / открытый API) | 8080 |
| iot-device (сервис устройств) | 8081 |
| iot-access (подключение / аутентификация) | 8082 |
| iot-data (сервис данных) | 8083 |
| iot-rule (движок правил) | 8084 |
| iot-cdn (управление CDN) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Использование модулей

- **Управление устройствами**: добавьте устройство в консоли → выберите OAuth вендора или прямой MQTT; онлайн-статус и жизненный цикл — в карточке устройства
- **Модель вещей**: определите атрибуты / события / сервисы для категорий устройств; панель клиента рендерится автоматически
- **Правила и оповещения**: настройте пороговые правила и автоматизацию сценариев; WebSocket-уведомления в реальном времени
- **Исторические данные**: iot-data хранит временные ряды; просмотр кривых и экспорт CSV / Excel
- **Отчёты и статистика**: многомерные отчёты по устройствам / данным / CDN / оповещениям / тенантам
- **Управление CDN**: настройка CDN нескольких вендоров, обновление / прогрев и подписанные URL
- **Подключение вендоров**: облачные OAuth-адаптеры Tuya / Xiaomi / Huawei / AWS / Azure

## Этапы внедрения

| Этап | Содержание |
|------|------|
| P0 Скелет | Структура репозитория, iot-gateway + iot-device + Docker-оркестрация (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Подключение | iot-access + адаптер Tuya + прямой MQTT |
| P2 Данные | iot-data + TDengine + исторические кривые |
| P3 Правила | iot-rule оповещения + автоматизация сценариев |
| P4 Вендоры + CDN | Адаптеры Xiaomi/Huawei/AWS/Azure + iot-cdn |
| P5 Фронтенд | apps/admin + apps/client полностью |
| P6 Запуск | Усиление безопасности, нагрузочное тестирование, OTA |

## Поддержка и донаты

Ваша поддержка — движущая сила проекта. Донаты приветствуются, огромное спасибо!

### Донат по QR-коду (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="QR-код WeChat Pay" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="QR-код Alipay" title="Alipay" />
</p>

WeChat Pay · Alipay

## Лицензия

Код этого проекта предоставлен только для обучения и обмена опытом.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
