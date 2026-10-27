# IoT प्लेटफ़ॉर्म

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="परियोजना शुभंकर" />
</p>

एक ऑल-इन-वन SaaS IoT प्लेटफ़ॉर्म जो घरेलू और विदेशी प्रमुख डिवाइस निर्माताओं (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT आदि) तक एकीकृत पहुंच प्रदान करता है, जिसमें डिवाइस प्रबंधन, थिंग मॉडल, नियम और अलर्ट, टाइम-सीरीज़ डेटा, CDN डिलीवरी और मल्टी-प्लेटफ़ॉर्म ऐप्स शामिल हैं। बैकएंड Rust माइक्रोसर्विस वर्कस्पेस (e-cat) है, फ्रंटएंड Flutter + HarmonyOS हैं, 13 भाषाओं के समर्थन के साथ।

## विशेषताएँ

- **मल्टी-वेंडर एक्सेस**: क्लाउड-टू-क्लाउड OAuth (Tuya / Xiaomi / Huawei / AWS / Azure) + डायरेक्ट MQTT (mTLS)
- **थिंग मॉडल**: प्रॉपर्टी / इवेंट / सर्विस का एकीकृत मॉडलिंग — एडमिन में मॉडलिंग, क्लाइंट में डायनामिक रेंडरिंग
- **डिवाइस जीवनचक्र**: रजिस्टर → सक्षम / अक्षम → अनबाइंड → डिलीट, रनटाइम पर ऑनलाइन / ऑफ़लाइन स्थिति
- **नियम और अलर्ट**: थ्रेशोल्ड नियम, सीन ऑटोमेशन, WebSocket रीयल-टाइम पुश
- **डेटा और रिपोर्ट**: TDengine टाइम-सीरीज़ स्टोरेज, हिस्ट्री कर्व्स, CSV/Excel निर्यात
- **CDN प्रबंधन**: वेंडर कॉन्फ़िग, सक्षम / अक्षम, रिफ्रेश और प्रीवार्म, साइन किए गए URL
- **मल्टी-टेनेंट SaaS**: टेनेंट आइसोलेशन, कोटा, रोल (admin / operator / read-only)
- **सुरक्षा**: security-rust इनबाउंड स्कैनिंग, JWT + RBAC, म्यूचुअल TLS, AES-एन्क्रिप्टेड क्रेडेंशियल, रेट लिमिटिंग और सर्किट ब्रेकर
- **13 भाषाओं में i18n**, Flutter Web / Mobile और नेटिव HarmonyOS (4 ऐप्स)

## आर्किटेक्चर

![आर्किटेक्चर](../architecture.hi.svg)

## डिवाइस एक्सेस फ़्लो

![फ़्लोचार्ट](../flow.hi.svg)

## फ़ीचर मैप

![फ़ीचर मैप](../features.hi.svg)

## डिवाइस जीवनचक्र

![जीवनचक्र](../lifecycle.hi.svg)

## सुरक्षा आर्किटेक्चर

![सुरक्षा](../security.hi.svg)

## टेक स्टैक

| परत | तकनीक |
|----|------|
| फ्रंटएंड | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| बैकएंड | Rust (axum · tokio · gRPC) · security-rust स्कैनिंग |
| मिडलवेयर | EMQX (MQTT) · Kafka (इवेंट बस) · gRPC (आंतरिक RPC) |
| स्टोरेज | MySQL 8 (मेटाडेटा) · TDengine (टाइम-सीरीज़) · Redis (शैडो / कैश) · S3 / MinIO (ऑब्जेक्ट) |
| एक्सेस | क्लाउड-टू-क्लाउड OAuth एडेप्टर · डायरेक्ट MQTT (mTLS) |

## रिपॉज़िटरी संरचना

```
├── apps/            # फ्रंटएंड ऐप्स
│   ├── admin/       # एडमिन कंसोल (Flutter + HarmonyOS)
│   └── client/      # क्लाइंट ऐप (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # दस्तावेज़, डायग्राम, दान छवियाँ
├── scripts/         # बिल्ड / वैलिडेशन / स्मोक-टेस्ट स्क्रिप्ट्स
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## एक-क्लिक इंस्टॉल

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

स्क्रिप्ट स्वचालित रूप से करती है: इंफ्रास्ट्रक्चर चालू करें (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → 6 व्यावसायिक सेवाएँ बनाएँ → .env कॉन्फ़िगरेशन बनाएँ → सेवा सूची और स्टार्ट कमांड प्रिंट करें। बार-बार चलाना सुरक्षित है।

## इंस्टॉलेशन गाइड

### पूर्वापेक्षाएँ

- Docker 24+ और docker compose (या docker-compose)
- Rust 1.80+ (stable, सेवाएँ संकलित करने के लिए; cargo इंस्टॉल होने पर स्क्रिप्ट स्वतः बिल्ड करती है)
- पोर्ट उपलब्धता जाँच: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 खाली होने चाहिए

### इंस्टॉलेशन चरण

1. इंफ्रास्ट्रक्चर स्थापित करें और चालू करें: `./scripts/install.sh` `docker compose up -d` चलाएगा
2. सेवाएँ बनाएँ: cargo मिलने पर स्क्रिप्ट स्वतः `cargo build --release` चलाती है; बाइनरी `scripts/bin/` में कॉपी होती हैं (या `e-cat/target/release/` के आउटपुट का उपयोग करें)
3. सेवाएँ चालू करें: स्क्रिप्ट के अंत में दिए गए कमांड से 6 सेवाएँ एक-एक करके चलाएँ
4. डेटाबेस माइग्रेशन: सेवा शुरू होने पर स्वतः चलता है; पहले स्टार्ट पर iot-access डिफ़ॉल्ट टेनेंट और एडमिन खाता बनाता है

## उपयोग

### लॉगिन

- एडमिन कंसोल: डिफ़ॉल्ट खाते `admin / admin123` (टेनेंट `tenant-1`) से लॉगिन करें

### सेवा पोर्ट

| सेवा | पोर्ट |
|------|------|
| iot-gateway (गेटवे / सार्वजनिक API) | 8080 |
| iot-device (डिवाइस सेवा) | 8081 |
| iot-access (कनेक्शन / प्रमाणीकरण) | 8082 |
| iot-data (डेटा सेवा) | 8083 |
| iot-rule (नियम इंजन) | 8084 |
| iot-cdn (CDN प्रबंधन) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### मॉड्यूल उपयोग

- **डिवाइस प्रबंधन**: कंसोल में डिवाइस जोड़ें → वेंडर OAuth या डायरेक्ट MQTT चुनें; विवरण में ऑनलाइन स्थिति और जीवनचक्र देखें
- **थिंग मॉडल**: डिवाइस श्रेणियों के लिए प्रॉपर्टी / इवेंट / सेवा परिभाषित करें; क्लाइंट कंट्रोल पैनल स्वतः रेंडर होता है
- **नियम और अलर्ट**: थ्रेशोल्ड नियम और सीन ऑटोमेशन कॉन्फ़िगर करें; ट्रिगर होने पर WebSocket रियल-टाइम पुश
- **ऐतिहासिक डेटा**: iot-data टाइम-सीरीज़ डेटा संग्रहीत करता है; हिस्ट्री कर्व देखें और CSV / Excel निर्यात करें
- **रिपोर्ट और आँकड़े**: डिवाइस / डेटा / CDN / अलर्ट / टेनेंट की बहुआयामी रिपोर्ट
- **CDN प्रबंधन**: मल्टी-वेंडर CDN कॉन्फ़िगर करें, रिफ्रेश / प्रीवॉर्म और साइन किए गए URL
- **मल्टी-वेंडर एक्सेस**: Tuya / Xiaomi / Huawei / AWS / Azure क्लाउड-टू-क्लाउड OAuth एडेप्टर

## कार्यान्वयन चरण

| चरण | दायरा |
|------|------|
| P0 स्केलेटन | रिपोज़िटरी संरचना, iot-gateway + iot-device + Docker ऑर्केस्ट्रेशन (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 एक्सेस | iot-access + Tuya एडेप्टर + डायरेक्ट MQTT |
| P2 डेटा | iot-data + TDengine + हिस्ट्री कर्व्स |
| P3 नियम | iot-rule अलर्ट + सीन ऑटोमेशन |
| P4 मल्टी-वेंडर + CDN | Xiaomi/Huawei/AWS/Azure एडेप्टर + iot-cdn |
| P5 फ्रंटएंड | apps/admin + apps/client पूर्ण |
| P6 लॉन्च | सुरक्षा सख्तीकरण, लोड टेस्टिंग, OTA |

## समर्थन और दान

आपका समर्थन ही परियोजना की निरंतर प्रगति का आधार है। दान का स्वागत है, बहुत-बहुत धन्यवाद!

### QR कोड स्कैन कर दान करें (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="WeChat Pay QR कोड" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="Alipay QR कोड" title="Alipay" />
</p>

WeChat Pay · Alipay

## लाइसेंस

इस परियोजना का कोड केवल सीखने और ज्ञान-साझाकरण के उद्देश्यों के लिए प्रदान किया गया है।


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
