import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';

/// 站内通知（/api/v1/rule/notifications）：列表 + 标记已读。
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

class NotificationsTab extends StatefulWidget {
  const NotificationsTab({super.key});

  @override
  State<NotificationsTab> createState() => _NotificationsTabState();
}

class _NotificationsTabState extends State<NotificationsTab> {
  int _version = 0;

  Future<List<AppNotification>> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/v1/rule/notifications');
    final list = resp is Map
        ? (resp['notifications'] ?? resp['items'] ?? resp['messages'])
        : resp;
    return parseList<AppNotification>(list, AppNotification.fromJson);
  }

  Future<void> _markRead(AppNotification n) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    try {
      await api.post('/api/v1/rule/notifications/${n.id}/read');
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
