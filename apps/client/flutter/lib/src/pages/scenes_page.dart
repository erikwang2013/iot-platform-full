import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../scene.dart';

/// 场景 + 站内通知：客户端只读展示，执行/已读走 rule 服务 REST。
/// 场景配置由管理端维护；实时告警仍在「告警」页（WS，见 alert_ws.dart）。
class ScenesPage extends StatelessWidget {
  const ScenesPage({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return DefaultTabController(
      length: 2,
      child: Scaffold(
        appBar: AppBar(
          title: Text(l10n.navScenes),
          bottom: TabBar(
            tabs: [
              Tab(text: l10n.adminSceneTab),
              Tab(text: l10n.adminNotificationTab),
            ],
          ),
        ),
        body: const TabBarView(children: [_ScenesTab(), _NotificationsTab()]),
      ),
    );
  }
}

class _ScenesTab extends StatefulWidget {
  const _ScenesTab();

  @override
  State<_ScenesTab> createState() => _ScenesTabState();
}

class _ScenesTabState extends State<_ScenesTab> {
  Future<List<Scene>> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/rule/scenes');
    final list = resp is Map ? (resp['scenes'] ?? resp['items']) : resp;
    return parseList<Scene>(list, Scene.fromJson);
  }

  String _condition(Map<String, dynamic> c, AppLocalizations l10n) =>
      switch ('${c['type']}') {
        'property' =>
          '${l10n.sceneTypeProperty} ${c['device_id']}:${c['code']} ${c['operator']} ${c['value']}',
        'time' => '${l10n.adminSceneTime} ${c['start']}–${c['end']}',
        _ => '$c',
      };

  String _action(Map<String, dynamic> a, AppLocalizations l10n) =>
      switch ('${a['type']}') {
        'command' =>
          '${l10n.sceneTypeCommand} ${a['device_id']}:${a['code']}=${a['value'] ?? ''}',
        'notify' => '${l10n.adminSceneNotify} ${a['title'] ?? ''}',
        _ => '$a',
      };

  String _resultType(String type, AppLocalizations l10n) => switch (type) {
        'command' => l10n.sceneTypeCommand,
        'notify' => l10n.adminSceneNotify,
        _ => type,
      };

  Future<void> _execute(Scene scene) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    try {
      final resp = await api.post('/api/rule/scenes/${scene.id}/execute');
      if (!mounted) return;
      final results =
          resp is Map && resp['results'] is List ? resp['results'] as List : const [];
      await showDialog<void>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: Text('${l10n.adminSceneExecute} · ${scene.name}'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final r in results.whereType<Map>())
                Text(
                  '${_resultType('${r['type']}', l10n)} · '
                  '${r['ok'] == true ? l10n.commonSuccess : l10n.commonFailed}'
                  '${r['detail'] == null ? '' : ' · ${r['detail']}'}',
                ),
              if (results.isEmpty) Text(l10n.commonEmpty),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(ctx),
              child: Text(l10n.commonOk),
            ),
          ],
        ),
      );
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('$e')));
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return ApiList<Scene>(
      load: _load,
      emptyText: l10n.commonEmpty,
      builder: (context, scenes) => ListView.builder(
        itemCount: scenes.length,
        itemBuilder: (context, i) {
          final s = scenes[i];
          return Card(
            margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: ListTile(
              isThreeLine: true,
              leading: Icon(
                s.enabled ? Icons.play_circle_outline : Icons.pause_circle_outline,
                color: s.enabled ? Colors.green : Colors.grey,
              ),
              title: Row(
                children: [
                  Expanded(child: Text(s.name.isEmpty ? s.id : s.name)),
                  Text(
                    s.enabled ? l10n.commonEnabled : l10n.sceneDisabled,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ],
              ),
              subtitle: Text(
                '${l10n.adminSceneConditions}: '
                '${s.conditions.map((c) => _condition(c, l10n)).join(' · ')}\n'
                '${l10n.adminSceneActions}: '
                '${s.actions.map((a) => _action(a, l10n)).join(' · ')}',
              ),
              trailing: FilledButton.tonal(
                onPressed: () => _execute(s),
                child: Text(l10n.adminSceneExecute),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// 站内通知（/api/rule/notifications）：列表 + 标记已读。
class AppNotification {
  AppNotification({
    required this.id,
    required this.title,
    required this.message,
    required this.read,
    required this.createdAt,
  });

  final String id;
  final String title;
  final String message;
  final bool read;
  final String createdAt;

  factory AppNotification.fromJson(Map<String, dynamic> j) => AppNotification(
        id: '${j['id'] ?? j['notification_id'] ?? ''}',
        title: '${j['title'] ?? j['message'] ?? j['body'] ?? ''}',
        message: '${j['message'] ?? j['body'] ?? j['content'] ?? ''}',
        read: j['read_at'] != null || j['read'] == true || j['is_read'] == true,
        createdAt: '${j['created_at'] ?? j['createdAt'] ?? j['ts'] ?? ''}',
      );
}

class _NotificationsTab extends StatefulWidget {
  const _NotificationsTab();

  @override
  State<_NotificationsTab> createState() => _NotificationsTabState();
}

class _NotificationsTabState extends State<_NotificationsTab> {
  int _version = 0;

  Future<List<AppNotification>> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/rule/notifications');
    final list = resp is Map
        ? (resp['notifications'] ?? resp['items'] ?? resp['messages'])
        : resp;
    return parseList<AppNotification>(list, AppNotification.fromJson);
  }

  Future<void> _markRead(AppNotification n) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    try {
      await api.post('/api/rule/notifications/${n.id}/read');
      if (!mounted) return;
      setState(() => _version++);
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(l10n.commonSuccess)));
    } catch (e) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('$e')));
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return ApiList<AppNotification>(
      key: ValueKey(_version),
      load: _load,
      emptyText: l10n.adminNotificationEmpty,
      builder: (context, notifications) => ListView.builder(
        itemCount: notifications.length,
        itemBuilder: (context, i) {
          final n = notifications[i];
          return Card(
            margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: ListTile(
              leading: Icon(
                n.read ? Icons.drafts_outlined : Icons.mark_email_unread_outlined,
                color: n.read ? Colors.grey : Theme.of(context).colorScheme.primary,
              ),
              title: Text(n.title),
              subtitle: Text(
                n.message == n.title || n.message.isEmpty
                    ? n.createdAt
                    : '${n.message}\n${n.createdAt}',
              ),
              trailing: n.read
                  ? null
                  : TextButton(
                      onPressed: () => _markRead(n),
                      child: Text(l10n.adminNotificationMarkRead),
                    ),
            ),
          );
        },
      ),
    );
  }
}
