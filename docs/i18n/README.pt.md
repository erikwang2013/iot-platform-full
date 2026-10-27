# Plataforma IoT

[中文](../../README.md) | [English](README.en.md) | [한국어](README.ko.md) | [Русский](README.ru.md) | [Deutsch](README.de.md) | [Français](README.fr.md) | [Español](README.es.md) | [Português](README.pt.md) | [हिन्दी](README.hi.md) | [العربية](README.ar.md) | [বাংলা](README.bn.md) | [Bahasa Indonesia](README.id.md) | [日本語](README.ja.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Mascote do projeto" />
</p>

Uma plataforma SaaS de IoT tudo-em-um que unifica o acesso aos principais fabricantes de dispositivos nacionais e internacionais (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT, etc.), oferecendo gestão de dispositivos, modelos de coisas, regras e alertas, dados de séries temporais, entrega via CDN e aplicativos multiplataforma. O backend é um workspace de microsserviços Rust (e-cat), os frontends são Flutter + HarmonyOS, com suporte a 13 idiomas.

## Recursos

- **Acesso multi-fabricante**: OAuth nuvem a nuvem (Tuya / Xiaomi / Huawei / AWS / Azure) + MQTT direto (mTLS)
- **Modelo de coisas**: modelagem unificada de propriedade / evento / serviço — modelado no admin, renderizado dinamicamente no cliente
- **Ciclo de vida do dispositivo**: registro → habilitar / desabilitar → desvincular → excluir, com status online / offline em tempo de execução
- **Regras e alertas**: regras de limite, automação de cenários, push WebSocket em tempo real
- **Dados e relatórios**: armazenamento de séries temporais TDengine, curvas históricas, exportação CSV/Excel
- **Gestão de CDN**: configuração de fornecedores, habilitar / desabilitar, refresh e prewarm, URLs assinadas
- **SaaS multi-tenant**: isolamento de inquilinos, cotas, funções (admin / operator / read-only)
- **Segurança**: varredura de entrada security-rust, JWT + RBAC, TLS mútuo, credenciais criptografadas com AES, limite de taxa e disjuntor
- **i18n em 13 idiomas**, Flutter Web / Mobile e HarmonyOS nativo (4 aplicativos)

## Arquitetura

![Arquitetura](../architecture.pt.svg)

## Fluxos de acesso de dispositivos

![Diagrama de fluxo](../flow.pt.svg)

## Mapa de recursos

![Mapa de recursos](../features.pt.svg)

## Ciclo de vida do dispositivo

![Ciclo de vida](../lifecycle.pt.svg)

## Arquitetura de segurança

![Segurança](../security.pt.svg)

## Pilha de tecnologia

| Camada | Tecnologia |
|----|------|
| Frontend | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Backend | Rust (axum · tokio · gRPC) · varredura security-rust |
| Middleware | EMQX (MQTT) · Kafka (barramento de eventos) · gRPC (RPC interno) |
| Armazenamento | MySQL 8 (metadados) · TDengine (séries temporais) · Redis (shadow / cache) · S3 / MinIO (objetos) |
| Acesso | Adaptadores OAuth nuvem a nuvem · MQTT direto (mTLS) |

## Estrutura do repositório

```
├── apps/            # Aplicativos frontend
│   ├── admin/       # Console de administração (Flutter + HarmonyOS)
│   └── client/      # Aplicativo cliente (Flutter + HarmonyOS)
├── e-cat/           # Rust 工作区（框架 + 业务微服务一体）
│   └── ecat*/       # 框架公共库 + 业务微服务（ecat · ecat-auth · ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-data-* …）
├── ../            # Documentação, diagramas, imagens de doações
├── scripts/         # Scripts de build / validação / smoke tests
└── docker-compose.yml  # 基础设施编排（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）
```

## Instalação em um clique

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

O script faz automaticamente: subir a infraestrutura (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → compilar os 6 serviços de negócio → gerar a configuração .env → imprimir a lista de serviços e os comandos de início. Pode ser executado novamente com segurança.

## Guia de instalação

### Pré-requisitos

- Docker 24+ e docker compose (ou docker-compose)
- Rust 1.80+ (stable, para compilar os serviços; o script compila automaticamente se o cargo estiver instalado)
- Verificação de portas: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 devem estar livres

### Etapas da instalação

1. Instalar e subir a infraestrutura: `./scripts/install.sh` executa `docker compose up -d`
2. Compilar os serviços: com cargo detectado, o script executa automaticamente `cargo build --release`; os binários vão para `scripts/bin/` (ou use os de `e-cat/target/release/`)
3. Iniciar os serviços: inicie os 6 serviços com os comandos impressos no final do script
4. Migração do banco: executada automaticamente na inicialização; no primeiro início o iot-access cria o locatário padrão e a conta de administrador

## Uso

### Login

- Console administrativo: entre com a conta `admin / admin123` (locatário `tenant-1`)

### Portas dos serviços

| Serviço | Porta |
|------|------|
| iot-gateway (gateway / API pública) | 8080 |
| iot-device (serviço de dispositivos) | 8081 |
| iot-access (conexão / autenticação) | 8082 |
| iot-data (serviço de dados) | 8083 |
| iot-rule (motor de regras) | 8084 |
| iot-cdn (gerenciamento de CDN) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Uso dos módulos

- **Gerenciamento de dispositivos**: adicione um dispositivo no console → escolha OAuth do fabricante ou MQTT direto; status online e ciclo de vida nos detalhes
- **Modelo de coisas**: defina propriedade / evento / serviço para as categorias; o painel do cliente é renderizado automaticamente
- **Regras e alertas**: configure regras de limite e automação de cenários; push via WebSocket em tempo real
- **Dados históricos**: o iot-data armazena séries temporais; curvas históricas e exportação CSV / Excel
- **Relatórios e estatísticas**: relatórios multidimensionais de dispositivos / dados / CDN / alertas / locatários
- **Gerenciamento de CDN**: configure CDN multi-fabricante, refresh / prewarm e URLs assinadas
- **Acesso multi-fabricante**: adaptadores OAuth nuvem a nuvem Tuya / Xiaomi / Huawei / AWS / Azure

## Fases de implementação

| Fase | Escopo |
|------|------|
| P0 Esqueleto | Estrutura do repo, iot-gateway + iot-device + orquestração Docker (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Acesso | iot-access + adaptador Tuya + MQTT direto |
| P2 Dados | iot-data + TDengine + curvas históricas |
| P3 Regras | alertas iot-rule + automação de cenários |
| P4 Multi-fabricante + CDN | Adaptadores Xiaomi/Huawei/AWS/Azure + iot-cdn |
| P5 Frontend | apps/admin + apps/client completos |
| P6 Lançamento | Endurecimento de segurança, testes de carga, OTA |

## Suporte e doações

Seu apoio impulsiona o projeto. Doações são bem-vindas, muito obrigado!

### Doar por QR code (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="QR code do WeChat Pay" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="QR code do Alipay" title="Alipay" />
</p>

WeChat Pay · Alipay

## Licença

O código deste projeto é fornecido apenas para fins de aprendizado e intercâmbio.


## 项目授权 / Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- 本项目仅供个人学习、研究使用。
- 未经作者书面授权，禁止任何形式的商业使用（包括但不限于商业部署、付费服务、二次销售、SaaS 运营及以本项目为基础提供商业服务）。
- 商业授权请联系 **erik@erik.xyz**。
- 未经授权的商业使用，作者将在全球范围内依法追究法律责任，并保留终身追责的权利。
- 第三方组件仍遵循其各自原有的许可证；视觉资产适用仓库内的 ASSETS-LICENSE.md（如适用）。

**English:** This project is provided for personal study and research only. Commercial use of any kind requires prior written authorization from the author (erik@erik.xyz). Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life. Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.
