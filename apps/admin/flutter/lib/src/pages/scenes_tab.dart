import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../scene.dart';
import '../value_parse.dart';

class ScenesTab extends StatefulWidget {
  const ScenesTab({super.key});

  @override
  State<ScenesTab> createState() => _ScenesTabState();
}

class _ScenesTabState extends State<ScenesTab> {
  static const _operators = ['gt', 'gte', 'lt', 'lte', 'eq', 'neq'];

  int _version = 0;
  int _seq = 0;

  String _rowKey() => 'r${_seq++}';

  Future<List<Scene>> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/rule/scenes');
    final list = resp is Map ? resp['scenes'] : resp;
    return parseList<Scene>(list, Scene.fromJson);
  }

  void _toast(String msg) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
  }

  String _textOf(dynamic v) {
    if (v == null) return '';
    if (v is String) return v;
    return jsonEncode(v);
  }

  Future<void> _toggle(Scene scene, bool enabled) async {
    final api = context.read<ApiClient>();
    try {
      await api.put('/api/rule/scenes/${scene.id}',
          body: scene.toJson(enabled: enabled));
      if (mounted) setState(() => _version++);
    } catch (e) {
      _toast('$e');
    }
  }

  Future<void> _execute(Scene scene) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    try {
      final resp = await api.post('/api/rule/scenes/${scene.id}/execute');
      if (!mounted) return;
      final result = resp is Map
          ? (resp['executed'] ?? resp['result'] ?? resp['message'] ?? resp['ok'])
          : null;
      _toast(result == null
          ? l10n.commonSuccess
          : l10n.adminResultValue('$result'));
    } catch (e) {
      _toast('$e');
    }
  }

  Future<void> _delete(Scene scene) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l10n.commonDelete),
        content: Text(l10n.adminSceneDeleteConfirm),
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
      await api.delete('/api/rule/scenes/${scene.id}');
      if (mounted) setState(() => _version++);
    } catch (e) {
      _toast('$e');
    }
  }

  Future<void> _edit(Scene? existing) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final name = TextEditingController(text: existing?.name ?? '');
    var enabled = existing?.enabled ?? true;
    final conditions = [
      for (final c in existing?.conditions ?? const <Map<String, dynamic>>[])
        Map<String, dynamic>.from(c)..['_k'] = _rowKey(),
    ];
    final actions = [
      for (final a in existing?.actions ?? const <Map<String, dynamic>>[])
        Map<String, dynamic>.from(a)..['_k'] = _rowKey(),
    ];
    if (conditions.isEmpty) {
      conditions.add({'type': 'property', '_k': _rowKey()});
    }
    if (actions.isEmpty) {
      actions.add({'type': 'command', '_k': _rowKey()});
    }
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(existing == null ? l10n.adminSceneCreate : l10n.adminSceneEdit),
        content: SizedBox(
          width: 420,
          child: StatefulBuilder(
            builder: (ctx, setDialogState) => SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  TextField(
                    controller: name,
                    decoration: InputDecoration(labelText: l10n.adminSceneName),
                  ),
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text(l10n.commonEnabled),
                    value: enabled,
                    onChanged: (v) => setDialogState(() => enabled = v),
                  ),
                  _sectionHeader(
                    l10n.adminSceneConditions,
                    l10n.adminSceneAddCondition,
                    () => setDialogState(() => conditions.add(
                        {'type': 'property', '_k': _rowKey()})),
                  ),
                  for (var i = 0; i < conditions.length; i++)
                    _conditionEditor(conditions, i, setDialogState),
                  _sectionHeader(
                    l10n.adminSceneActions,
                    l10n.adminSceneAddAction,
                    () => setDialogState(
                        () => actions.add({'type': 'command', '_k': _rowKey()})),
                  ),
                  for (var i = 0; i < actions.length; i++)
                    _actionEditor(actions, i, setDialogState),
                ],
              ),
            ),
          ),
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: Text(l10n.commonCancel)),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: Text(l10n.commonSave)),
        ],
      ),
    );
    if (ok != true) {
      name.dispose();
      return;
    }
    final body = {
      'name': name.text.trim(),
      'enabled': enabled,
      'conditions': [
        for (final c in conditions)
          if (c['type'] == 'time')
            {
              'type': 'time',
              'start': '${c['start'] ?? ''}',
              'end': '${c['end'] ?? ''}',
            }
          else
            {
              'type': 'property',
              'device_id': '${c['device_id'] ?? ''}',
              'code': '${c['code'] ?? ''}',
              'operator': '${c['operator'] ?? 'gt'}',
              'value': c['value'] ?? '',
            },
      ],
      'actions': [
        for (final a in actions)
          if (a['type'] == 'notify')
            {
              'type': 'notify',
              'title': '${a['title'] ?? ''}',
              'message': '${a['message'] ?? ''}',
            }
          else
            {
              'type': 'command',
              'device_id': '${a['device_id'] ?? ''}',
              'code': '${a['code'] ?? ''}',
              'value': a['value'] ?? '',
            },
      ],
    };
    name.dispose();
    try {
      if (existing == null) {
        await api.post('/api/rule/scenes', body: body);
      } else {
        await api.put('/api/rule/scenes/${existing.id}', body: body);
      }
      if (!mounted) return;
      setState(() => _version++);
      _toast(l10n.commonSuccess);
    } catch (e) {
      _toast('$e');
    }
  }

  Widget _sectionHeader(String label, String addLabel, VoidCallback onAdd) {
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Row(
        children: [
          Expanded(
              child: Text(label,
                  style: Theme.of(context).textTheme.titleSmall)),
          TextButton.icon(
            onPressed: onAdd,
            icon: const Icon(Icons.add),
            label: Text(addLabel),
          ),
        ],
      ),
    );
  }

  Widget _field(
    String id,
    String initial,
    String label,
    ValueChanged<String> onChanged,
  ) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: TextFormField(
        key: ValueKey(id),
        initialValue: initial,
        decoration: InputDecoration(
            labelText: label, isDense: true, border: const OutlineInputBorder()),
        onChanged: onChanged,
      ),
    );
  }

  Widget _conditionEditor(
      List<Map<String, dynamic>> list, int i, StateSetter setDialogState) {
    final l10n = AppLocalizations.of(context)!;
    final c = list[i];
    final key = '${c['_k']}';
    final isTime = c['type'] == 'time';
    return Card(
      margin: const EdgeInsets.symmetric(vertical: 4),
      child: Padding(
        padding: const EdgeInsets.all(8),
        child: Column(
          children: [
            Row(
              children: [
                Expanded(
                  child: DropdownButtonFormField<String>(
                    key: ValueKey('type-$key'),
                    initialValue: isTime ? 'time' : 'property',
                    isDense: true,
                    decoration: InputDecoration(
                        labelText: l10n.adminSceneConditionType,
                        isDense: true),
                    items: [
                      DropdownMenuItem(
                          value: 'property', child: Text(l10n.modelProperty)),
                      DropdownMenuItem(
                          value: 'time', child: Text(l10n.adminSceneTime)),
                    ],
                    onChanged: (v) =>
                        setDialogState(() => c['type'] = v ?? 'property'),
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.delete_outline),
                  // 后端要求 conditions/actions 各 1..=20 条，保留最后一行
                  onPressed: list.length > 1
                      ? () => setDialogState(() => list.removeAt(i))
                      : null,
                ),
              ],
            ),
            if (isTime)
              Row(
                children: [
                  Expanded(
                      child: _field('start-$key', '${c['start'] ?? ''}',
                          '${l10n.adminSceneStart} (HH:MM)',
                          (v) => c['start'] = v)),
                  const SizedBox(width: 8),
                  Expanded(
                      child: _field('end-$key', '${c['end'] ?? ''}',
                          '${l10n.adminSceneEnd} (HH:MM)',
                          (v) => c['end'] = v)),
                ],
              )
            else ...[
              _field('device-$key', '${c['device_id'] ?? ''}', l10n.ruleDeviceId,
                  (v) => c['device_id'] = v),
              Row(
                children: [
                  Expanded(
                      child: _field('code-$key', '${c['code'] ?? ''}',
                          l10n.ruleCode, (v) => c['code'] = v)),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 110,
                    child: DropdownButtonFormField<String>(
                      key: ValueKey('op-$key'),
                      initialValue:
                          _operators.contains(c['operator']) ? '${c['operator']}' : 'gt',
                      isDense: true,
                      decoration: InputDecoration(
                          labelText: l10n.ruleOperator, isDense: true),
                      items: [
                        for (final op in _operators)
                          DropdownMenuItem(value: op, child: Text(op)),
                      ],
                      onChanged: (v) => c['operator'] = v ?? 'gt',
                    ),
                  ),
                ],
              ),
              _field('value-$key', _textOf(c['value']), l10n.adminCommandValue,
                  (v) => c['value'] = parseScalar(v)),
            ],
          ],
        ),
      ),
    );
  }

  Widget _actionEditor(
      List<Map<String, dynamic>> list, int i, StateSetter setDialogState) {
    final l10n = AppLocalizations.of(context)!;
    final a = list[i];
    final key = '${a['_k']}';
    final isNotify = a['type'] == 'notify';
    return Card(
      margin: const EdgeInsets.symmetric(vertical: 4),
      child: Padding(
        padding: const EdgeInsets.all(8),
        child: Column(
          children: [
            Row(
              children: [
                Expanded(
                  child: DropdownButtonFormField<String>(
                    key: ValueKey('atype-$key'),
                    initialValue: isNotify ? 'notify' : 'command',
                    isDense: true,
                    decoration: InputDecoration(
                        labelText: l10n.adminSceneActionType, isDense: true),
                    items: [
                      DropdownMenuItem(
                          value: 'command',
                          child: Text(l10n.adminCommandTitle)),
                      DropdownMenuItem(
                          value: 'notify', child: Text(l10n.adminSceneNotify)),
                    ],
                    onChanged: (v) =>
                        setDialogState(() => a['type'] = v ?? 'command'),
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.delete_outline),
                  onPressed: list.length > 1
                      ? () => setDialogState(() => list.removeAt(i))
                      : null,
                ),
              ],
            ),
            if (isNotify) ...[
              _field('title-$key', '${a['title'] ?? ''}', l10n.adminSceneTitle,
                  (v) => a['title'] = v),
              _field('message-$key', '${a['message'] ?? ''}',
                  l10n.adminSceneMessage, (v) => a['message'] = v),
            ] else ...[
              _field('adevice-$key', '${a['device_id'] ?? ''}',
                  l10n.ruleDeviceId, (v) => a['device_id'] = v),
              _field('acode-$key', '${a['code'] ?? ''}', l10n.ruleCode,
                  (v) => a['code'] = v),
              _field('avalue-$key', _textOf(a['value']), l10n.adminCommandValue,
                  (v) => a['value'] = parseScalar(v)),
            ],
          ],
        ),
      ),
    );
  }

  String _summary(Scene s) {
    final parts = <String>[
      for (final c in s.conditions)
        c['type'] == 'time'
            ? '${c['start'] ?? ''}-${c['end'] ?? ''}'
            : '${c['device_id'] ?? ''}.${c['code'] ?? ''} '
                '${c['operator'] ?? ''} ${c['value'] ?? ''}',
      for (final a in s.actions)
        a['type'] == 'notify'
            ? '${a['title'] ?? ''}'
            : '${a['device_id'] ?? ''}.${a['code'] ?? ''}=${a['value'] ?? ''}',
    ];
    return parts.join(' · ');
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
          child: Align(
            alignment: Alignment.centerRight,
            child: FilledButton.icon(
              onPressed: () => _edit(null),
              icon: const Icon(Icons.add),
              label: Text(l10n.adminSceneCreate),
            ),
          ),
        ),
        Expanded(
          child: ApiList<Scene>(
            key: ValueKey(_version),
            load: _load,
            emptyText: l10n.commonEmpty,
            builder: (context, scenes) => ListView.builder(
              itemCount: scenes.length,
              itemBuilder: (context, i) {
                final s = scenes[i];
                return Card(
                  margin:
                      const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
                  child: ListTile(
                    title: Text(s.name),
                    subtitle: Text(_summary(s),
                        maxLines: 2, overflow: TextOverflow.ellipsis),
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Switch(
                          value: s.enabled,
                          onChanged: (v) => _toggle(s, v),
                        ),
                        IconButton(
                          tooltip: l10n.adminSceneExecute,
                          icon: const Icon(Icons.play_arrow),
                          onPressed: () => _execute(s),
                        ),
                        PopupMenuButton<String>(
                          onSelected: (v) =>
                              v == 'edit' ? _edit(s) : _delete(s),
                          itemBuilder: (ctx) => [
                            PopupMenuItem(
                                value: 'edit', child: Text(l10n.commonEdit)),
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
        ),
      ],
    );
  }
}
