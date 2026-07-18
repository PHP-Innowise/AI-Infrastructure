---
name: filament
description: Build Filament admin panels on Laravel - Resources, Schemas (Forms/Infolists), Tables, Relation Managers, Actions, and Widgets backed by Eloquent models and Policies. Use for internal/admin CRUD surfaces; use coder-frontend for customer-facing UI.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, browser-verify]
related: [coder-frontend, coder, architecture-implementer, database-designer]
---

# Filament

## Overview

Implement or extend a Filament admin panel: Resources (CRUD over an Eloquent model), Schemas (Forms/Infolists), Tables, Relation Managers, Actions, custom Pages, and Widgets. Filament is a server-driven UI framework built on Livewire, Alpine.js, and Tailwind - panels are defined almost entirely in PHP.

Targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). Filament v5 requires Livewire v4; Filament v4 requires Livewire v3 - the application code is otherwise identical between the two major versions, so check `composer.json` before assuming which one a project is on.

## Scope Boundary

`filament` owns **admin-panel / internal-tooling CRUD surfaces** built with the Filament package. Use a sibling skill when the task is different:

- **Customer-facing UI** (public pages, checkout, marketing pages) in Blade, plain Livewire, or Inertia -> use `coder-frontend`, even if the project also has a Filament panel elsewhere.
- **Scaffolding a brand-new panel/resource skeleton from an architecture decision** (which resources exist, what a module boundary looks like) -> `architecture-implementer` decides the skeleton; `filament` fills in the fields, columns, and business logic once the skeleton exists (or you can do both in one pass for a small, well-understood resource).
- **Designing the underlying Eloquent schema** (tables, relationships, indexes) -> `database-designer` first if the data model isn't settled; `filament` then builds the panel on top of it.
- **Non-Filament API endpoints** for the same resource (e.g. a public REST API alongside an admin panel) -> `api-designer`/`coder`.

## Locate The Panel

- Panel providers: `app/Providers/Filament/*PanelProvider.php` (registered in `bootstrap/providers.php`). Confirm the panel's `path()`, `authGuard()`, and `discoverResources()`/`discoverPages()`/`discoverWidgets()` directories before assuming conventional locations.
- Resources: `app/Filament/Resources/<Model>/` containing `<Model>Resource.php`, a `Pages/` directory (`ListX`, `CreateX`, `EditX`, sometimes `ViewX`), a `Schemas/` directory (`<Model>Form.php`, and an `<Model>Infolist.php` if a dedicated view page exists), and a `Tables/` directory (`<Model>Table.php`).
- Relation managers: `app/Filament/Resources/<Model>/RelationManagers/`.
- Widgets: `app/Filament/Widgets/` (dashboard) or `app/Filament/Resources/<Model>/Widgets/` (resource-scoped).
- Custom actions shared across resources: `app/Filament/Actions/`.

Use `php artisan make:filament-resource <Model> --generate` to scaffold a resource from an existing model's schema (infers field/column types from the migration) rather than hand-writing every field from scratch.

## Resource Pattern

```php
// app/Filament/Resources/Invitations/InvitationResource.php
namespace App\Filament\Resources\Invitations;

use App\Filament\Resources\Invitations\Pages;
use App\Filament\Resources\Invitations\Schemas\InvitationForm;
use App\Filament\Resources\Invitations\Tables\InvitationsTable;
use App\Models\Invitation;
use Filament\Resources\Resource;
use Filament\Schemas\Schema;
use Filament\Tables\Table;

class InvitationResource extends Resource
{
    protected static ?string $model = Invitation::class;

    protected static ?string $navigationIcon = 'heroicon-o-envelope';

    public static function form(Schema $schema): Schema
    {
        return InvitationForm::configure($schema);
    }

    public static function table(Table $table): Table
    {
        return InvitationsTable::configure($table);
    }

    public static function getRelations(): array
    {
        return [];
    }

    public static function getPages(): array
    {
        return [
            'index' => Pages\ListInvitations::route('/'),
            'create' => Pages\CreateInvitation::route('/create'),
            'edit' => Pages\EditInvitation::route('/{record}/edit'),
        ];
    }
}
```

```php
// app/Filament/Resources/Invitations/Schemas/InvitationForm.php
namespace App\Filament\Resources\Invitations\Schemas;

use Filament\Forms\Components\DateTimePicker;
use Filament\Forms\Components\Select;
use Filament\Forms\Components\TextInput;
use Filament\Schemas\Schema;

class InvitationForm
{
    public static function configure(Schema $schema): Schema
    {
        return $schema->components([
            TextInput::make('email')->email()->required()->unique(ignoreRecord: true),
            Select::make('role')->options(['player' => 'Player', 'parent' => 'Parent'])->required(),
            DateTimePicker::make('expires_at')->required()->native(false),
        ]);
    }
}
```

```php
// app/Filament/Resources/Invitations/Tables/InvitationsTable.php
namespace App\Filament\Resources\Invitations\Tables;

use Filament\Actions\DeleteAction;
use Filament\Actions\EditAction;
use Filament\Tables\Columns\TextColumn;
use Filament\Tables\Filters\SelectFilter;
use Filament\Tables\Table;

class InvitationsTable
{
    public static function configure(Table $table): Table
    {
        return $table
            ->columns([
                TextColumn::make('email')->searchable()->sortable(),
                TextColumn::make('role')->badge(),
                TextColumn::make('expires_at')->dateTime()->sortable(),
            ])
            ->filters([SelectFilter::make('role')->options(['player' => 'Player', 'parent' => 'Parent'])])
            ->recordActions([EditAction::make(), DeleteAction::make()]);
    }
}
```

Since v4, all actions (`EditAction`, `DeleteAction`, form/table/page actions) live under the unified `Filament\Actions` namespace - do not import the older per-package action classes from v3-era code or tutorials.

## Relation Managers

Use a Relation Manager when a resource needs to manage a related model's records inline (e.g. a `Team`'s `Players`) rather than navigating to a separate resource:

```php
// app/Filament/Resources/Teams/RelationManagers/PlayersRelationManager.php
class PlayersRelationManager extends RelationManager
{
    protected static string $relationship = 'players';

    public function form(Schema $schema): Schema { /* fields */ }
    public function table(Table $table): Table { /* columns + attach/detach or create/edit actions */ }
}
```

Register it in the parent resource's `getRelations()`. Prefer a Relation Manager over duplicating a full resource when the related records only make sense in the context of their parent.

## Authorization

Filament resources call into the model's Policy automatically (`viewAny`, `create`, `update`, `delete`, etc.) when one exists - write/keep the Policy (`php artisan make:policy InvitationPolicy --model=Invitation`) rather than hiding actions with ad-hoc visibility checks. Use `canAccess()`/`shouldRegisterNavigation()` on the Resource for panel-level visibility, but always back it with the Policy for the actual authorization decision; navigation hiding alone is not authorization (see `AGENTS.md`).

## Custom Pages And Widgets

- Custom (non-CRUD) pages: `php artisan make:filament-page`, extend `Filament\Pages\Page`, and give them a Blade view when the built-in schema components aren't the right fit (e.g. a settings page backed by a config-driven form).
- Dashboard/stat widgets: `php artisan make:filament-widget`, extend `StatsOverviewWidget`, `ChartWidget`, or `TableWidget`. Keep widget queries efficient (aggregate/cache where the underlying dataset is large) since dashboard widgets run on every panel load.

## Testing Filament Resources

Filament ships Livewire-based testing helpers - test through `livewire()` against the resource's page components, not by hitting routes directly:

```php
use function Pest\Livewire\livewire;

it('can create an invitation', function () {
    livewire(CreateInvitation::class)
        ->fillForm(['email' => 'a@example.com', 'role' => 'player'])
        ->call('create')
        ->assertHasNoFormErrors();

    expect(Invitation::where('email', 'a@example.com')->exists())->toBeTrue();
});
```

Cover: the happy-path create/edit, a validation-failure case (`assertHasFormErrors(['field'])`), and an authorization-denied case for a user without the relevant Policy permission.

## Verification

Possible checks:

```bash
php artisan test --filter=Invitation
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan filament:optimize   # caches components/icons for production
```

Use only commands present in the project.

## Final Output

Return what changed (resource/schema/table/relation-manager/widget files), how authorization is enforced, tests run, Context Summary, and next step.
