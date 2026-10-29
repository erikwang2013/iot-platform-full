import 'dart:convert';

/// 表单文本 → JSON 标量：可作为 JSON 解析时用解析结果（bool/num/带引号字符串），
/// 否则按原始字符串发送（后端 command/scene 的 value 字段为任意 JSON）。
dynamic parseScalar(String raw) {
  final text = raw.trim();
  if (text.isEmpty) return '';
  try {
    return jsonDecode(text);
  } catch (_) {
    return raw;
  }
}
