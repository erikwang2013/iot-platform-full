import 'dart:async';
import 'dart:convert';

import 'package:web_socket_channel/web_socket_channel.dart';

import 'models.dart';

/// 连接 iot-rule 的实时告警通道。
/// 网关不代理 WebSocket（见 ecat-gateway main.rs），前端直连 rule 服务
/// （默认 8084）。浏览器 WS 无法设置 Authorization header，JWT 经
/// Sec-WebSocket-Protocol 子协议传递（`bearer, <token>`），不放进 URL。
Stream<AlertMessage> alertStream(String wsBaseUrl, String token) {
  final channel = WebSocketChannel.connect(
    Uri.parse('$wsBaseUrl/ws'),
    protocols: ['bearer', token],
  );
  return channel.stream
      .map((raw) => raw is String ? raw : utf8.decode(raw as List<int>))
      .map((s) => AlertMessage.fromJsonString(s));
}
