/// 场景（/api/v1/rule/scenes）：条件 + 动作，客户端只读展示与手动执行。
class Scene {
  Scene({
    required this.id,
    required this.name,
    required this.enabled,
    required this.conditions,
    required this.actions,
  });

  final String id;
  final String name;
  final bool enabled;
  final List<Map<String, dynamic>> conditions;
  final List<Map<String, dynamic>> actions;

  factory Scene.fromJson(Map<String, dynamic> j) => Scene(
        id: '${j['id'] ?? j['scene_id'] ?? ''}',
        name: '${j['name'] ?? ''}',
        enabled: j['enabled'] != false,
        conditions: _maps(j['conditions']),
        actions: _maps(j['actions']),
      );

  static List<Map<String, dynamic>> _maps(dynamic v) => v is List
      ? v
          .whereType<Map<String, dynamic>>()
          .map(Map<String, dynamic>.from)
          .toList()
      : const [];
}
