terraform {
  required_providers {
    azurerm = {
      version = "3.75.0"
      source  = "hashicorp/azurerm"
    }
    azapi = {
      source  = "Azure/azapi"
      version = "1.9.0"
    }
  }
}

provider "azurerm" {
  features {}
}

locals {
  eln = join("-", [var.environment, var.location_short, var.name])
}

data "azurerm_client_config" "current" {}

resource "azurerm_resource_group" "this" {
  name     = "rg-${local.eln}"
  location = var.location
}

resource "azurerm_log_analytics_workspace" "this" {
  name                = "log-${local.eln}"
  location            = azurerm_resource_group.this.location
  resource_group_name = azurerm_resource_group.this.name
  sku                 = "PerGB2018"
  retention_in_days   = 30
}

resource "azurerm_storage_account" "this" {
  name                     = join("", [var.environment, var.location_short, var.name])
  resource_group_name      = azurerm_resource_group.this.name
  location                 = azurerm_resource_group.this.location
  account_tier             = "Premium"
  account_replication_type = "LRS"
  account_kind             = "FileStorage"
}

resource "azurerm_storage_share" "this" {
  name                 = "kitops-state"
  quota                = "100"
  storage_account_name = azurerm_storage_account.this.name
}

resource "azurerm_key_vault" "app" {
  name                      = "kv-${local.eln}"
  location                  = azurerm_resource_group.this.location
  resource_group_name       = azurerm_resource_group.this.name
  purge_protection_enabled  = false
  enable_rbac_authorization = true
  sku_name                  = "standard"
  tenant_id                 = data.azurerm_client_config.current.tenant_id
}

resource "azurerm_role_assignment" "current_user_key_vault" {
  scope                = azurerm_key_vault.app.id
  role_definition_name = "Key Vault Secrets Officer"
  principal_id         = data.azurerm_client_config.current.object_id
}

resource "azurerm_role_assignment" "app_key_vault" {
  scope                = azurerm_key_vault.app.id
  role_definition_name = "Key Vault Secrets User"
  principal_id         = data.azurerm_container_app.app.identity[0].principal_id
}

# resource "azurerm_key_vault_secret" "ssh_private_key" {
#   name         = "ssh-private-key"
#   value        = "replace-me"
#   key_vault_id = azurerm_key_vault.app.id
#   tags = {
#     file-encoding = "utf-8"
#   }

#   depends_on = [azurerm_role_assignment.current_user_key_vault]
#   lifecycle {
#     ignore_changes = [value]
#   }
# }

resource "azurerm_container_app_environment" "this" {
  name                       = "cae-${local.eln}"
  location                   = azurerm_resource_group.this.location
  resource_group_name        = azurerm_resource_group.this.name
  log_analytics_workspace_id = azurerm_log_analytics_workspace.this.id
  # internal_load_balancer_enabled = false
}

resource "azurerm_container_app_environment_storage" "example" {
  name                         = "caes-${local.eln}"
  container_app_environment_id = azurerm_container_app_environment.this.id
  account_name                 = azurerm_storage_account.this.name
  share_name                   = azurerm_storage_share.this.name
  access_key                   = azurerm_storage_account.this.primary_access_key
  access_mode                  = "ReadWrite"
}

data "azurerm_container_app" "app" {
  name                = "ca-${local.eln}"
  resource_group_name = azurerm_resource_group.this.name
  depends_on          = [azapi_resource.app]
}

resource "azapi_resource" "app" {
  type      = "Microsoft.App/jobs@2023-05-01"
  name      = "ca-${local.eln}"
  parent_id = azurerm_resource_group.this.id

  body = jsonencode({
    location = azurerm_resource_group.this.location
    properties = {
      environmentId = azurerm_container_app_environment.this.id
      configuration = {
        scheduleTriggerConfig = {
          cronExpression         = "* * * * *"
          parallelism            = 1
          replicaCompletionCount = 1
        }
        replicaRetryLimit = 1
        replicaTimeout    = 1800
        triggerType       = "Schedule"
        # secrets = [
        #   {
        #     name        = "ssh-private-key"
        #     keyVaultUrl = "${azurerm_key_vault.app.vault_uri}secrets/ssh-private-key"
        #     identity    = "system"
        #   },
        # ]
      }
      template = {
        containers = [
          {
            name  = "kitops",
            image = "bittrance/kitops:0.1.0"
            resources = {
              cpu    = 0.25
              memory = "0.5Gi"
            }
            args = [
              "--state-file", "/state/state.yaml",
              "--repo-dir", "/state",
              "--url", "git@github.com:bittrance/kitops.git",
              "--action", "/bin/ls",
              # "--action", "cd terraform/ && terraform init && terraform apply -auto-approve",
            ]
            volumeMounts = [
              {
                volumeName = "kitops-state"
                mountPath  = "/state"
              },
              # {
              #   volumeName = "ssh"
              #   mountPath  = "/root/.ssh"
              # },
            ]
          }
        ]
        volumes = [
          {
            name        = "kitops-state"
            storageName = "caes-${local.eln}" # bug -> azurerm_container_app_environment_storage.example.name
            storageType = "AzureFile"
          },
          # {
          #   name        = "ssh"
          #   storageType = "Secret"
          #   secrets = [
          #     {
          #       secretRef = "ssh-private-key"
          #       path      = "id_rsa"
          #     }
          #   ]

          # },
        ]
      }
    }
    identity = {
      type = "SystemAssigned"
    }
  })
}
