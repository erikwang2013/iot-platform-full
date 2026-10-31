import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';

/// CDN 管理：供应商配置 CRUD、启停、连通测试、刷新预热任务、签名 URL。
class CdnPage extends StatelessWidget {
  const CdnPage({super.key});

  static const _vendors = ['aliyun', 'tencent', 'cloudflare'];

  Future<List<CdnVendor>> _load(BuildContext context) async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/cdn/providers');
    final list = resp is Map ? resp['providers'] : resp;
    return parseList<CdnVendor>(list, CdnVendor.fromJson);
  }

  Future<void> _addVendor(BuildContext context) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final name = TextEditingController();
    final domain = TextEditingController();
    final config = TextEditingController(text: '{}');
    String vendor = _vendors.first;
    String? configError;
    final payload = await showDialog<Map<String, dynamic>>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setDialogState) => AlertDialog(
          title: Text(l10n.cdnAddVendor),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                  controller: name,
                  decoration: const InputDecoration(labelText: 'Name')),
              DropdownButtonFormField<String>(
                initialValue: vendor,
                decoration: InputDecoration(labelText: l10n.cdnType),
                items: [
                  for (final v in _vendors)
                    DropdownMenuItem(value: v, child: Text(v)),
                ],
                onChanged: (v) => setDialogState(() => vendor = v!),
              ),
              TextField(
                  controller: domain,
                  decoration: InputDecoration(labelText: l10n.cdnDomain)),
              TextField(
                controller: config,
                maxLines: 4,
                decoration: InputDecoration(
                    labelText: 'Config (JSON)', errorText: configError),
              ),
            ],
          ),
          actions: [
            TextButton(
                onPressed: () => Navigator.pop(ctx),
                child: Text(l10n.commonCancel)),
            FilledButton(
              onPressed: () {
                final raw = config.text.trim();
                dynamic cfg;
                try {
                  cfg = raw.isEmpty ? <String, dynamic>{} : jsonDecode(raw);
                } on FormatException {
                  setDialogState(() => configError = 'Invalid JSON');
                  return;
                }
                Navigator.pop(ctx, <String, dynamic>{
                  'name': name.text.trim(),
                  'vendor': vendor,
                  'domain': domain.text.trim(),
                  'config': cfg,
                });
              },
              child: Text(l10n.commonSave),
            ),
          ],
        ),
      ),
    );
    if (payload == null) return;
    try {
      await api.post('/api/cdn/providers', body: payload);
      _toast(context, l10n.commonSuccess);
    } catch (e) {
      _toast(context, '$e');
    }
  }

  Future<void> _toggle(BuildContext context, CdnVendor v, bool enabled) async {
    final api = context.read<ApiClient>();
    try {
      await api.post(
          '/api/cdn/providers/${v.id}/${enabled ? 'enable' : 'disable'}');
    } catch (e) {
      _toast(context, '$e');
    }
  }

  Future<void> _test(BuildContext context, CdnVendor v) async {
    final api = context.read<ApiClient>();
    try {
      final resp = await api.post('/api/cdn/providers/${v.id}/test');
      _toast(context, '${l10n(context).cdnTest}: ${resp ?? 'ok'}');
    } catch (e) {
      _toast(context, '$e');
    }
  }

  /// action 为后端动作名：purge（刷新）/ prefetch（预热）。
  Future<void> _task(BuildContext context, CdnVendor v, String action) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final url = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(action == 'purge' ? l10n.cdnRefresh : l10n.cdnPurge),
        content: TextField(
            controller: url,
            decoration: InputDecoration(labelText: l10n.cdnUrlHint)),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: Text(l10n.commonCancel)),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: Text(l10n.commonConfirm)),
        ],
      ),
    );
    if (ok != true) return;
    try {
      await api.post('/api/cdn/providers/${v.id}/$action',
          body: {
            'urls': [url.text.trim()]
          });
      _toast(context, l10n.commonSuccess);
    } catch (e) {
      _toast(context, '$e');
    }
  }

  Future<void> _signedUrl(BuildContext context, CdnVendor v) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final url = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l10n.cdnSignedUrl),
        content: TextField(
            controller: url,
            decoration: InputDecoration(labelText: l10n.cdnUrlHint)),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: Text(l10n.commonCancel)),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: Text(l10n.commonConfirm)),
        ],
      ),
    );
    if (ok != true) return;
    try {
      final resp = await api.post('/api/cdn/providers/${v.id}/signed-url',
          body: {'url': url.text.trim(), 'expires_secs': 3600});
      final signed = resp is Map ? '${resp['url'] ?? ''}' : '${resp ?? ''}';
      await showDialog<void>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: Text(l10n.cdnSignedUrl),
          content: SelectableText('${l10n.cdnSignedUrlResult}\n$signed'),
          actions: [
            TextButton(
                onPressed: () => Navigator.pop(ctx),
                child: Text(l10n.commonConfirm)),
          ],
        ),
      );
    } catch (e) {
      _toast(context, '$e');
    }
  }

  Future<void> _delete(BuildContext context, CdnVendor v) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l10n.commonDelete),
        content: Text('${v.name} · ${v.vendor}'),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: Text(l10n.commonCancel)),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: Text(l10n.commonConfirm)),
        ],
      ),
    );
    if (confirmed != true) return;
    try {
      await api.delete('/api/cdn/providers/${v.id}');
      _toast(context, l10n.commonSuccess);
    } catch (e) {
      _toast(context, '$e');
    }
  }

  void _toast(BuildContext context, String msg) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
  }

  AppLocalizations l10n(BuildContext context) => AppLocalizations.of(context)!;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return Scaffold(
      appBar: AppBar(title: Text(l10n.navCdn)),
      body: ApiList<CdnVendor>(
        load: () => _load(context),
        emptyText: l10n.commonEmpty,
        builder: (context, vendors) => ListView.builder(
          itemCount: vendors.length,
          itemBuilder: (context, i) {
            final v = vendors[i];
            return Card(
              margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
              child: ListTile(
                leading: const Icon(Icons.cloud_outlined),
                title: Text('${v.name} · ${v.vendor}'),
                subtitle: Text('${v.domain} · ${v.status}'),
                trailing: Wrap(
                  spacing: 0,
                  children: [
                    Switch(
                        value: v.enabled, onChanged: (e) => _toggle(context, v, e)),
                    PopupMenuButton<String>(
                      onSelected: (a) => switch (a) {
                        'test' => _test(context, v),
                        'refresh' => _task(context, v, 'purge'),
                        'purge' => _task(context, v, 'prefetch'),
                        'signed' => _signedUrl(context, v),
                        _ => _delete(context, v),
                      },
                      itemBuilder: (ctx) => [
                        PopupMenuItem(value: 'test', child: Text(l10n.cdnTest)),
                        PopupMenuItem(
                            value: 'refresh', child: Text(l10n.cdnRefresh)),
                        PopupMenuItem(
                            value: 'purge', child: Text(l10n.cdnPurge)),
                        PopupMenuItem(
                            value: 'signed', child: Text(l10n.cdnSignedUrl)),
                        PopupMenuItem(
                            value: 'delete', child: Text(l10n.commonDelete)),
                      ],
                    ),
                  ],
                ),
              ),
            );
          },
        ),
      ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: () => _addVendor(context),
        icon: const Icon(Icons.add),
        label: Text(l10n.cdnAddVendor),
      ),
    );
  }
}
