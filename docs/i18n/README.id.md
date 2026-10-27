# Platform IoT

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Maskot Proyek" />
</p>

Platform SaaS IoT satu atap yang menyatukan akses ke vendor perangkat utama dalam dan luar negeri (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT, dll.), menyediakan manajemen perangkat, model benda, aturan & peringatan, data deret waktu, distribusi CDN, dan aplikasi multi-platform. Backend adalah workspace mikroservis Rust (e-cat), frontend Flutter + HarmonyOS, mendukung 13 bahasa.

## Fitur

- **Integrasi multi-vendor**: OAuth cloud-to-cloud (Tuya / Xiaomi / Huawei / AWS / Azure) + MQTT langsung (mTLS)
- **Model benda**: pemodelan terpadu properti / peristiwa / layanan — dimodelkan di admin, dirender dinamis di klien
- **Siklus hidup perangkat**: daftar → aktif / nonaktif → lepas tautan → hapus, dengan status online / offline saat berjalan
- **Aturan & peringatan**: aturan ambang batas, otomasi skenario, push WebSocket real-time
- **Data & laporan**: penyimpanan deret waktu TDengine, kurva historis, ekspor CSV/Excel
- **Manajemen CDN**: konfigurasi vendor, aktif/nonaktif, refresh & prewarm, URL bertanda tangan
- **SaaS multi-tenant**: isolasi tenant, kuota, peran (admin / operator / read-only)
- **Keamanan**: pemindaian inbound security-rust, JWT + RBAC, TLS dua arah, kredensial terenkripsi AES, pembatasan laju & circuit breaker
- **i18n 13 bahasa**, di Flutter Web / Mobile dan HarmonyOS native (4 aplikasi)

## Arsitektur

![Arsitektur](../architecture.id.svg)

## Alur Akses Perangkat

![Diagram Alur](../flow.id.svg)

## Peta Fitur

![Peta Fitur](../features.id.svg)

## Siklus Hidup Perangkat

![Siklus Hidup](../lifecycle.id.svg)

## Arsitektur Keamanan

![Keamanan](../security.id.svg)

## Tumpukan Teknologi

| Lapisan | Teknologi |
|----|------|
| Frontend | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Backend | Rust (axum · tokio · gRPC) · pemindaian security-rust |
| Middleware | EMQX (MQTT) · Kafka (bus peristiwa) · gRPC (RPC internal) |
| Penyimpanan | MySQL 8 (metadata) · TDengine (deret waktu) · Redis (shadow / cache) · S3 / MinIO (objek) |
| Akses | Adaptor OAuth cloud-to-cloud · MQTT langsung (mTLS) |

## Struktur Repositori

```
├── apps/            # Aplikasi frontend
│   ├── admin/       # Konsol admin (Flutter + HarmonyOS)
│   └── client/      # Aplikasi klien (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # Dokumen, diagram, gambar donasi
├── scripts/         # Skrip build / validasi / smoke test
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## Instalasi Sekali Klik

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

Skrip otomatis: menjalankan infrastruktur (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → membangun 6 layanan bisnis → membuat konfigurasi .env → mencetak daftar layanan dan perintah menjalankannya. Aman dijalankan berulang kali.

## Panduan Instalasi

### Prasyarat

- Docker 24+ dan docker compose (atau docker-compose)
- Rust 1.80+ (stable, untuk mengompilasi layanan; skrip membangun otomatis jika cargo terpasang)
- Periksa ketersediaan port: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 harus kosong

### Langkah Instalasi

1. Pasang dan jalankan infrastruktur: `./scripts/install.sh` menjalankan `docker compose up -d`
2. Bangun layanan: jika cargo terdeteksi, skrip otomatis menjalankan `cargo build --release`; biner disalin ke `scripts/bin/` (atau gunakan hasil di `e-cat/target/release/`)
3. Jalankan layanan: mulai 6 layanan satu per satu dengan perintah yang dicetak di akhir skrip
4. Migrasi basis data: berjalan otomatis saat layanan dimulai; pada start pertama, iot-access membuat tenant default dan akun admin

## Penggunaan

### Login

- Konsol admin: masuk dengan akun default `admin / admin123` (tenant `tenant-1`)

### Port Layanan

| Layanan | Port |
|------|------|
| iot-gateway (gateway / API publik) | 8080 |
| iot-device (layanan perangkat) | 8081 |
| iot-access (koneksi / autentikasi) | 8082 |
| iot-data (layanan data) | 8083 |
| iot-rule (mesin aturan) | 8084 |
| iot-cdn (manajemen CDN) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Penggunaan Modul

- **Manajemen perangkat**: tambah perangkat di konsol → pilih OAuth vendor atau MQTT langsung; status online dan siklus hidup di detail
- **Model benda**: definisikan properti / event / layanan untuk kategori perangkat; panel kontrol klien dirender otomatis
- **Aturan & peringatan**: atur aturan ambang dan otomatisasi skenario; push WebSocket real-time saat terpicu
- **Data historis**: iot-data menyimpan data deret waktu; lihat kurva historis dan ekspor CSV / Excel
- **Laporan & statistik**: laporan multidimensi perangkat / data / CDN / peringatan / tenant
- **Manajemen CDN**: konfigurasi CDN multi-vendor, refresh / prewarm dan URL bertanda tangan
- **Akses multi-vendor**: adaptor OAuth cloud-to-cloud Tuya / Xiaomi / Huawei / AWS / Azure

## Fase Implementasi

| Fase | Cakupan |
|------|------|
| P0 Kerangka | Struktur repo, iot-gateway + iot-device + orkestrasi Docker (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Akses | iot-access + adaptor Tuya + MQTT langsung |
| P2 Data | iot-data + TDengine + kurva historis |
| P3 Aturan | peringatan iot-rule + otomasi skenario |
| P4 Multi-vendor + CDN | Adaptor Xiaomi/Huawei/AWS/Azure + iot-cdn |
| P5 Frontend | apps/admin + apps/client semua platform |
| P6 Rilis | Penguatan keamanan, uji beban, OTA |

## Dukungan & Donasi

Dukungan Anda adalah kekuatan bagi kelanjutan proyek ini. Donasi sangat kami hargai!

### Pindai untuk Donasi (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="Kode QR WeChat Pay" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="Kode QR Alipay" title="Alipay" />
</p>

WeChat Pay · Alipay

## Lisensi

Kode proyek ini disediakan hanya untuk tujuan pembelajaran dan komunikasi.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
