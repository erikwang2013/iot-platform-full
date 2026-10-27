# منصة إنترنت الأشياء

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="تعويذة المشروع" />
</p>

منصة SaaS شاملة لإنترنت الأشياء توحّد الوصول إلى كبرى شركات تصنيع الأجهزة المحلية والعالمية (Tuya وXiaomi وHuawei وAWS IoT وAzure IoT وغيرها)، وتوفر إدارة الأجهزة ونماذج الأشياء والقواعد والتنبيهات وبيانات السلاسل الزمنية وتوزيع CDN وتطبيقات متعددة المنصات. الواجهة الخلفية هي مساحة عمل للخدمات المصغرة بلغة Rust (e-cat)، والواجهات الأمامية هي Flutter + HarmonyOS، مع دعم 13 لغة.

## المزايا

- **الوصول متعدد الشركات**: OAuth من السحابة إلى السحابة (Tuya / Xiaomi / Huawei / AWS / Azure) + MQTT مباشر (mTLS)
- **نموذج الأشياء**: نمذجة موحدة للخاصية / الحدث / الخدمة — النمذجة في لوحة الإدارة، والعرض الديناميكي في العميل
- **دورة حياة الجهاز**: تسجيل ← تمكين / تعطيل ← إلغاء الربط ← حذف، مع حالة متصل / غير متصل وقت التشغيل
- **القواعد والتنبيهات**: قواعد العتبة، أتمتة السيناريوهات، دفع WebSocket في الوقت الفعلي
- **البيانات والتقارير**: تخزين السلاسل الزمنية TDengine، منحنيات السجل، تصدير CSV/Excel
- **إدارة CDN**: إعداد المورد، تمكين / تعطيل، التحديث والتسخين، عناوين URL موقعة
- **SaaS متعدد المستأجرين**: عزل المستأجرين، الحصص، الأدوار (admin / operator / read-only)
- **الأمان**: فحص وارد security-rust، JWT + RBAC، TLS ثنائي الاتجاه، بيانات اعتماد مشفرة AES، تحديد المعدل وقاطع الدائرة
- **i18n بـ 13 لغة**، Flutter Web / Mobile و HarmonyOS الأصلي (4 تطبيقات)

## البنية المعمارية

![البنية المعمارية](../architecture.ar.svg)

## تدفقات وصول الأجهزة

![مخطط التدفق](../flow.ar.svg)

## خريطة المزايا

![خريطة المزايا](../features.ar.svg)

## دورة حياة الجهاز

![دورة الحياة](../lifecycle.ar.svg)

## بنية الأمان

![الأمان](../security.ar.svg)

## حزمة التقنيات

| الطبقة | التقنية |
|----|------|
| الواجهة الأمامية | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| الواجهة الخلفية | Rust (axum · tokio · gRPC) · فحص security-rust |
| الوسيط | EMQX (MQTT) · Kafka (ناقل الأحداث) · gRPC (RPC داخلي) |
| التخزين | MySQL 8 (بيانات وصفية) · TDengine (سلاسل زمنية) · Redis (ظل / مخبأ) · S3 / MinIO (كائنات) |
| الوصول | محولات OAuth من سحابة إلى سحابة · MQTT مباشر (mTLS) |

## هيكل المستودع

```
├── apps/            # تطبيقات الواجهة الأمامية
│   ├── admin/       # وحدة التحكم الإدارية (Flutter + HarmonyOS)
│   └── client/      # تطبيق العميل (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # الوثائق والرسوم البيانية وصور التبرعات
├── scripts/         # نصوص البناء / التحقق / اختبارات الدخان
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## التثبيت بنقرة واحدة

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

يقوم السكربت تلقائيًا بـ: تشغيل البنية التحتية (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) ← بناء الخدمات الست ← إنشاء إعدادات .env ← طباعة قائمة الخدمات وأوامر التشغيل. التشغيل المتكرر آمن.

## دليل التثبيت

### المتطلبات الأساسية

- Docker 24+ و docker compose (أو docker-compose)
- Rust 1.80+ (stable، لبناء الخدمات؛ يقوم السكربت بالبناء تلقائيًا إذا كان cargo مثبتًا)
- فحص المنافذ: 8080-8085 و 3306 و 6379 و 1883 و 9092 و 9000-9001 و 6041 يجب أن تكون شاغرة

### خطوات التثبيت

1. تثبيت وتشغيل البنية التحتية: سيقوم `./scripts/install.sh` بتنفيذ `docker compose up -d`
2. بناء الخدمات: عند اكتشاف cargo، يشغّل السكربت تلقائيًا `cargo build --release`، وتُنسخ الملفات التنفيذية إلى `scripts/bin/` (أو استخدم نواتج `e-cat/target/release/`)
3. تشغيل الخدمات: شغّل الخدمات الست واحدة تلو الأخرى بالأوامر المطبوعة في نهاية السكربت
4. ترحيل قاعدة البيانات: يتم تلقائيًا عند تشغيل الخدمة؛ عند أول تشغيل، ينشئ iot-access المستأجر الافتراضي وحساب المسؤول

## الاستخدام

### تسجيل الدخول

- لوحة الإدارة: سجّل الدخول بالحساب الافتراضي `admin / admin123` (المستأجر `tenant-1`)

### منافذ الخدمات

| الخدمة | المنفذ |
|------|------|
| iot-gateway (البوابة / API عام) | 8080 |
| iot-device (خدمة الأجهزة) | 8081 |
| iot-access (الاتصال / المصادقة) | 8082 |
| iot-data (خدمة البيانات) | 8083 |
| iot-rule (محرك القواعد) | 8084 |
| iot-cdn (إدارة CDN) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### استخدام الوحدات

- **إدارة الأجهزة**: أضف جهازًا في لوحة الإدارة ← اختر OAuth للمورّد أو MQTT مباشر؛ حالة الاتصال ودورة الحياة في التفاصيل
- **نموذج الأشياء**: عرّف الخاصية / الحدث / الخدمة لفئات الأجهزة؛ تظهر لوحة العميل تلقائيًا
- **القواعد والتنبيهات**: اضبط قواعد العتبة وأتمتة السيناريوهات؛ إشعار WebSocket فوري عند التفعيل
- **البيانات التاريخية**: يخزن iot-data البيانات الزمنية؛ عرض المنحنيات التاريخية وتصدير CSV / Excel
- **التقارير والإحصاءات**: تقارير متعددة الأبعاد للأجهزة / البيانات / CDN / التنبيهات / المستأجرين
- **إدارة CDN**: تكوين CDN متعدد المورّدين، تحديث / تسخين وعناوين URL موقعة
- **الوصول متعدد المورّدين**: محولات OAuth من سحابة إلى سحابة لـ Tuya / Xiaomi / Huawei / AWS / Azure

## مراحل التنفيذ

| المرحلة | النطاق |
|------|------|
| P0 الهيكل | هيكل المستودع، iot-gateway + iot-device + تنسيق Docker (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 الوصول | iot-access + محول Tuya + MQTT مباشر |
| P2 البيانات | iot-data + TDengine + منحنيات السجل |
| P3 القواعد | تنبيهات iot-rule + أتمتة السيناريوهات |
| P4 متعدد الشركات + CDN | محولات Xiaomi/Huawei/AWS/Azure + iot-cdn |
| P5 الواجهة الأمامية | apps/admin + apps/client بالكامل |
| P6 الإطلاق | تقوية الأمان، اختبار الحمل، OTA |

## الدعم والتبرعات

دعمكم هو قوة دفع استمرار المشروع. التبرعات مرحب بها، شكرًا جزيلاً!

### التبرع بمسح رمز QR (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="رمز QR لـ WeChat Pay" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="رمز QR لـ Alipay" title="Alipay" />
</p>

WeChat Pay · Alipay

## الترخيص

يُقدَّم كود هذا المشروع لأغراض التعلم وتبادل المعرفة فقط.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
